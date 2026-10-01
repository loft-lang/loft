// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! What code DISTURBS: the containers a body grows, removes from or reshapes, directly or through
//! a `&` parameter of a callee, closed over the call graph — and the sentence that tells the author
//! why a view had to be copied out of its container.

use super::places::{
    ParamPlace, base_container_place, call_arg_place, compose_param_place, peel_stack_ref,
};
use crate::data::{Data, Type, Value};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::use_analysis::ANY_FIELD;
use crate::variables::Function;

/// Tell the author that a view was COPIED out of its container, in the sentence its cause
/// earns.
///
/// One home for the three sentences, because the copy itself has two mechanisms and the report
/// must not differ between them: the deps STRIP materialises a plain projection bind, and the
/// per-ARM lift materialises a branch- or discharge-valued one.  A reader cannot tell which
/// route their binding took, and `(H-Materialise)`'s promise — "the author is told" — is about
/// the copy, not about how it was arranged.
/// `via` is the CALLEE that did it, where this frame's own statements did not (@PLN164 C3).
/// It is the same principle the `Grown` arm's note below already states: a reader sent looking
/// for a statement that is not in the function pays for the difference, and a disturbance one
/// frame down is exactly that — nothing in `c_grow_callee` appends to `sc`.
pub(super) fn report_materialised_view(
    cause: ViewCause,
    vname: &str,
    cname: &str,
    fname: &str,
    via: Option<&str>,
) {
    // The advice is the author's to read, so a payload binding is named as they wrote it.
    let vname = crate::variables::author_spelling(vname);
    let vname = vname.as_str();
    match cause {
        ViewCause::Reshaped => {
            crate::copy_manifest::note_materialised_view(vname, cname, fname, via);
        }
        // loft#1373 — the fourth invalidator: the container GREW, so the elements may
        // have moved to a larger record. Same materialise, different sentence: a
        // reader told "removing an element renumbers the others" goes looking for a
        // `remove` that is not in the function.
        ViewCause::Grown => {
            crate::copy_manifest::note_grown_view(vname, cname, fname, via);
        }
        // @PLN130 F8 — the third invalidator: the container VARIABLE is reassigned,
        // so the dep still names `bx` while the store it named is gone. Different
        // cause, different way out, so a distinct advice line.
        ViewCause::Reassigned => {
            crate::copy_manifest::note_reassigned_view(vname, cname, fname, via);
        }
    }
}

/// The user-facing name of the callee a disturbance travelled through, if it did.
pub(super) fn disturbance_via(data: &Data, cause: &Disturbance) -> Option<String> {
    cause
        .via
        .filter(|d| *d != u32::MAX)
        .map(|d| data.def(d).original_name().clone())
}

/// Every container variable `code` RESHAPES — removes from, or grows.
///
/// Detects two of @FR-B-Disturb's four place-ending events: REMOVING from a container and
/// GROWING one.  (The other two are RE-KEYING an element and REASSIGNING the container
/// itself, which [`established_stores`] answers.  OVERWRITING a place does not disturb it —
/// the write lands in the place the view already points at, and `v[i] = x` lowers to
/// `OpCopyRecord`, which is named nowhere here.)
///
/// `v.remove(i)` lowers to `OpRemoveVector(v, size, index)` and the in-loop `e#remove` to
/// `OpRemove(index, container, …)` — the one op naming its container at arg 1, where every
/// other names it at arg 0.  Both renumber the positions inside the container's store, which
/// is what invalidates an element view.
///
/// GROWING is the fourth event, and the one this list was short by (loft#1373).  A vector
/// that outgrows its allocation is copied into a larger record — `Store::resize` answers a
/// NEW record number when it cannot absorb the free block beside it, `vector_append` repoints
/// the container's handle at it, and the old record is freed — so every element moves and a
/// view bound before the growth names an address the elements have left.  Measured: `d: S =
/// v[0]` followed by two hundred appends read `4294967296` on both backends with strict
/// stores silent, while the same code with TWO appends read `1`, because nothing had
/// reallocated yet.  One shape, two answers, decided by an allocation the author cannot see —
/// which is why ANY growth disturbs rather than only one that provably crosses the capacity.
///
/// The five growth spellings, each naming its container at arg 0: `OpNewRecord` (a
/// single-element append and a keyed add — it calls `vector_append`), `OpPreAllocVector` (the
/// capacity request a literal and a sized append make), `OpAppendVector` (`v += w`),
/// `OpInsertVector` (an insert, which also shifts every later element) and `OpHashAdd` (which
/// can rehash and move records).  A container's own CONSTRUCTION uses these too and needs no
/// separate test: the walk runs in ORDER and only shakes views that are already open, so an
/// op running before any view of that container exists reaches nothing.
///
/// Only a container named by a plain `Var` is collected; a reshape reached through some
/// other expression is not recognised, so the answer is a lower bound and a missed case
/// keeps today's behaviour rather than inventing a new one.
/// **This frame only.** What a CALLEE disturbs is [`disturbed_params_map`]'s answer, unioned in
/// beside this one by [`ViewWalk::disturb_via_calls`] — kept apart because the two are read off
/// different bodies, not because the events differ.
///
/// The half that is still deliberately absent is the `&` PARAMETER VIEW: a callee holding
/// `target: &Box` while another parameter reshapes the container around it writes through a
/// reference that no longer reaches its source, and the write is silently lost (@PLN130 probe
/// 38 cell A1, [loft#779](https://github.com/loft-lang/loft/issues/779)). A parameter has no
/// bind in the callee, so there is nothing to materialise AT, and copying behind the author's
/// back is not what `&` asked for (@PLN87) — the decided answer there is to REFUSE the program,
/// which is `reshape_refusals`' side of this file.
pub(super) fn reshaped_containers(
    code: &Value,
    data: &Data,
    function: &Function,
) -> HashSet<(u16, u32)> {
    let mut out = places_named_by(code, data, &|name| match name {
        "OpRemove" => Some(1),
        "OpRemoveVector" | "OpKeepVectorRange" => Some(0),
        _ => None,
    });
    // @FR-Col-RemoveDense — a KEYED removal reaches all five keyed kinds through ONE op, and
    // only one of them renumbers, so this has to be keyed on the KIND and not on the op.
    //
    // `@FR-Col-RemoveKeyed`: `hash`, `index`, `spatial` and `trie` give each element a record
    // of its own, so removing one leaves every other key reachable AT THE SAME ADDRESS —
    // measured, a view of another element reads correctly after the removal and a write
    // through it still lands, which is why collecting them here would materialise a binding
    // whose write is fine today.  A `sorted` is the INLINE keyed kind: its elements sit in key
    // order in one dense array, so a removal shifts every later position exactly as a
    // vector's does, and `@FR-Col-RemoveDense` names the two by-value kinds together.
    //
    // The same split `Stores::remove_vector_at`'s `is_linked` gate makes for the LEAK half of
    // this rule (loft#1402): one `sorted` leaked through `#remove` and not through
    // `[key] = null`, and here it goes stale through `[key] = null` where `hash` does not.
    // Two symptoms, one boundary.
    //
    // Read off the container VARIABLE's type, so a `sorted` reached through a FIELD is not
    // collected — a lower bound, kept because the projection carries its element type rather
    // than its collection kind and guessing there would shake the dense kinds' siblings.
    for place in places_named_by(code, data, &|name| match name {
        "OpHashRemove" => Some(0),
        _ => None,
    }) {
        if place.1 != ANY_FIELD {
            continue;
        }
        // `sorted` is collected WHOLE, for the reason above: it is the inline kind, so a
        // removal shifts every later position and ends every place the container holds.
        //
        // The record-per-element kinds are collected too, and that is loft#1460 — leaving
        // them out entirely meant a `&` view of the record a removal FREES was never
        // refused, and a later insert reusing that record read the stale write (measured on
        // both backends: `c = &h[30]; h[30] = null; h[70] = …; c.tag = 999` put 999 into
        // k70).  What keeps this from becoming the over-approximation the comment above
        // rejects is `shake_places`, which drops a view whose OWN key is a literal differing
        // from the removal's: `c = h[2]; h[1] = null` keeps aliasing, `c = &h[30];
        // h[30] = null` does not.  Collecting without that filter was measured and is
        // strictly wrong — it materialises the corpus's four `…_is_not_a_reshape` controls
        // and turns a plain view's write into a lost one, which trades one silent-wrong for
        // another.
        if matches!(
            function.tp(place.0).base(),
            Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _)
                | Type::Trie(_, _, _)
        ) {
            out.insert(place);
        }
    }
    out
}

/// Every container variable `code` GROWS — @FR-B-Disturb's fourth place-ending event.
///
/// Apart from [`reshaped_containers`] because the ADVICE differs, and only because of that:
/// a removal renumbers the elements after the one removed, while a growth can move ALL of
/// them to a larger record, so a reader told the wrong one goes looking for a `remove` that
/// is not there.  Both shake the same views for the same reason.
///
/// The five spellings, each naming its container at arg 0: `OpNewRecord` (a single-element
/// append and a keyed add — it calls `vector_append`), `OpPreAllocVector` (the capacity
/// request a literal and a sized append make), `OpAppendVector` (`v += w`), `OpInsertVector`
/// (an insert, which also shifts every later element) and `OpHashAdd` (which can rehash and
/// move records).
pub(super) fn grown_containers(
    code: &Value,
    data: &Data,
    function: &Function,
    database: Option<&crate::database::Stores>,
    cleared: &HashSet<(u16, u32)>,
) -> HashSet<(u16, u32)> {
    let mut out: HashSet<(u16, u32)> = HashSet::default();
    code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let name = data.def(*d).name();
        // @PLN157 § V-m — a fused scalar append (`OpPush<Kind>`) grows its container: the
        // variable itself in the plain form, a FIELD of it when the container is the field
        // access `OpGetField(var, off, _)` — the same two answers `OpNewRecord` gives below.
        if name.starts_with("OpPush") {
            match args.first().map(Value::unspan) {
                Some(Value::Var(c)) => {
                    out.insert((*c, ANY_FIELD));
                }
                Some(Value::Call(g, gargs))
                    if data.def(*g).name() == "OpGetField"
                        && let Some(Value::Var(c)) = gargs.first().map(Value::unspan)
                        && let Some(Value::Int(off)) = gargs.get(1).map(Value::unspan)
                        && let Ok(off) = u32::try_from(*off)
                        && !cleared.contains(&(*c, off)) =>
                {
                    out.insert((*c, off));
                }
                _ => {}
            }
            return;
        }
        if !matches!(
            name,
            "OpNewRecord" | "OpPreAllocVector" | "OpAppendVector" | "OpInsertVector" | "OpHashAdd"
        ) {
            return;
        }
        // `OpNewRecord(parent, tp, fld)` names its container in TWO parts, and only the
        // whole-variable form belongs here: `fld == u16::MAX` is an append to the variable
        // itself (`v += [x]` emits `OpNewRecord(v, tp, 65535)`), while any other `fld` is an
        // append to that variable's FIELD (`s.us_redo += [x]` emits `OpNewRecord(s, tp, 1)`).
        //
        // Collecting the parent for the second shape shakes every view rooted at the same
        // variable whichever field it names.  Measured on `moros_editor`'s `undo_pop`:
        // `e = s.us_entries[idx]` was materialised because the SIBLING field `s.us_redo` grew,
        // so each undo entry was read out of a copy, the stack silently stopped recording, and
        // `undo_depth` answered 0 where 3 was due.  A field-qualified growth is left
        // UNCOLLECTED rather than compared field-wise, which keeps this function the lower
        // bound its sibling's doc claims: a missed disturbance costs a materialise, a spurious
        // one costs a program its meaning.
        let Some(Value::Var(c)) = args.first().map(Value::unspan) else {
            return;
        };
        // `OpNewRecord(parent, tp, fld)` names its container in TWO parts: `fld == u16::MAX`
        // is an append to the variable itself (`v += [x]`), any other `fld` an append to that
        // variable's FIELD (`w.a += [x]`).  The field is a NUMBER and a view carries a byte
        // OFFSET, so the two are converted here — `Stores::field_position` against the
        // parent's own struct type — and the place is `(w, offset)`.
        //
        // Without the store the conversion cannot be made, and the field-qualified growth is
        // left UNCOLLECTED rather than widened to the parent: reading the parent alone shakes
        // every view rooted at it whichever field it names, which silently emptied
        // `moros_editor`'s undo stack when loft#1373 first shipped.  A missed disturbance
        // costs a materialise; a spurious one costs a program its meaning.
        let mut place = ANY_FIELD;
        if name == "OpNewRecord" {
            let fld = match args.get(2).map(Value::unspan) {
                Some(Value::Int(f)) => *f,
                _ => return,
            };
            if fld != i32::from(u16::MAX) {
                let Some(db) = database else { return };
                let parent = data.type_def_nr(function.tp(*c).base());
                if parent == u32::MAX {
                    return;
                }
                let off = db.field_position(data.def(parent).known_type(), fld as u16);
                if off == u16::MAX {
                    return;
                }
                place = u32::from(off);
                if cleared.contains(&(*c, place)) {
                    return;
                }
            }
        }
        out.insert((*c, place));
    });
    out
}

/// The PLACES `code` disturbs at the argument `which` picks, for the ops it picks.
///
/// A whole VARIABLE ends every place inside it (`ANY_FIELD`); a FIELD of one ends the places
/// inside THAT field only.  The distinction is the whole reason this answers places rather
/// than variables: `p.va.remove(0)` and `p.vb.remove(0)` both name `p`, and treating either as
/// "everything in `p`" materialises a view whose write lands today.  `grown_containers` records
/// the measurement — collecting the PARENT for a field-qualified growth shook every view rooted
/// at the same variable, and `moros_editor`'s undo stack silently stopped recording.
///
/// Before loft#1401's matrix this collected ONLY a plain `Var`, so a removal reached through a
/// field was not a disturbance at all: `c = p.va[1]; p.va.remove(0)` read the element that
/// shifted in, on both backends and in silence, where the same code with `va` in a local
/// materialises and says so.  The VIEW side already named the place `(p, off_va)` —
/// [`value_view_place`] resolves a projection chain to its outermost field — so only this half
/// was short and the two never met.
///
/// A container reached through anything else — a call result, an element of an element — is
/// still uncollected, and that stays the lower bound it always was: a missed disturbance costs
/// a materialise, a spurious one costs a program its meaning.
fn places_named_by(
    code: &Value,
    data: &Data,
    which: &dyn Fn(&str) -> Option<usize>,
) -> HashSet<(u16, u32)> {
    let mut out: HashSet<(u16, u32)> = HashSet::default();
    code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let Some(at) = which(data.def(*d).name()) else {
            return;
        };
        let Some(arg) = args.get(at) else { return };
        if let Some(place) = named_place(arg, data) {
            out.insert(place);
        }
    });
    out
}

/// The PLACE an op argument names — a bare variable is the WHOLE variable, anything else is
/// asked of [`base_container_place`].
///
/// One home because two readers need the same answer and the bare-`Var` case is the one a
/// second reader forgets: `OpHashRemove(h, …)` passes its container as a plain `Value::Var`,
/// and `base_container_place` alone answers `None` for that (it resolves PROJECTIONS), so a
/// reader built on it silently sees no removals at all.  Measured — loft#1460's key filter
/// spared nothing until both sides asked this.
pub(super) fn named_place(arg: &Value, data: &Data) -> Option<(u16, u32)> {
    match arg.unspan() {
        Value::Var(c) => Some((*c, ANY_FIELD)),
        other => base_container_place(other, data),
    }
}

/// @PLN130 F9 — which `&` parameters of `d_nr` its body REMOVES from.
///
/// The mirror of [`reassigned_ref_params`], and needed for the same reason: a `&` parameter
/// is a double indirection into the CALLER's variable, so `all.remove(0)` in the callee
/// renumbers the caller's container. Nothing at the call site says so, which is why the
/// caller needs this fact about the callee rather than a guess from the argument's shape
/// ([loft#779](https://github.com/loft-lang/loft/issues/779)).
///
/// Answered from the callee's IR: an `OpRemoveVector` / `OpRemove` whose container argument
/// is an argument slot declared `RefVar`. Argument slots lead the variable numbering, so the
/// slot number indexes the attribute list. This is the DIRECT answer only; a removal further
/// down reaches the caller through [`removed_params_map`], which closes it over the call graph.
fn removed_ref_params(data: &Data, d_nr: u32) -> HashSet<u16> {
    let def = data.def(d_nr);
    let mut out: HashSet<u16> = HashSet::default();
    if def.attributes.is_empty() {
        return out;
    }
    def.code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let arg = match data.def(*d).name() {
            "OpRemoveVector" | "OpKeepVectorRange" => args.first(),
            "OpRemove" => args.get(1),
            _ => return,
        };
        if let Some(Value::Var(slot)) = arg.map(Value::unspan)
            && let Some(a) = def.attributes.get(usize::from(*slot))
            && matches!(a.typedef, Type::RefVar(_))
        {
            out.insert(*slot);
        }
    });
    out
}

/// Every user definition that removes from at least one of its `&` parameters, and which.
///
/// Built once per program rather than re-derived at each call site: the question is asked once
/// per CALL, and a callee body would otherwise be re-walked once per call to it.
pub(super) type RemovedParams = HashMap<u32, HashSet<u16>>;

/// [`RemovedParams`], CLOSED OVER THE CALL GRAPH: a function that forwards its own `&`
/// parameter to something that removes from it removes from it too.
///
/// Without the closure the answer is one frame deep, and the hole is one an author would trip
/// over by refactoring: extracting `all.remove(0)` into a helper makes the refusal disappear and
/// the silent lost write come back (probe 40 cell X7). Closed with a worklist over
/// *"caller `c` passes its own `&` parameter `s` as callee `e`'s parameter `k`"* edges, built in
/// the same pass as the direct removals, so the cost stays one walk of each body plus the
/// propagation.
///
/// A callee reached only through a runtime fn-ref has no edge here and keeps today's behaviour —
/// a lower bound in the safe direction, since the refusal simply does not fire.
pub(super) fn removed_params_map(data: &Data) -> RemovedParams {
    let mut out = RemovedParams::default();
    let mut work: Vec<(u32, u16)> = Vec::new();
    // (callee, its param) -> every (caller, caller's own `&` param) that feeds it.
    let mut forwards: HashMap<(u32, u16), Vec<(u32, u16)>> = HashMap::default();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !def.name.starts_with("n_") {
            continue;
        }
        for k in removed_ref_params(data, d_nr) {
            if out.entry(d_nr).or_default().insert(k) {
                work.push((d_nr, k));
            }
        }
        if def.attributes.is_empty() {
            continue;
        }
        def.code.walk(&mut |v| {
            let Value::Call(callee, args) = v else { return };
            if !data.def(*callee).name.starts_with("n_") {
                return;
            }
            for (i, arg) in args.iter().enumerate() {
                let Ok(i) = u16::try_from(i) else { continue };
                // Only the caller's OWN `&` parameter forwards a reshape upwards; a local
                // container passed down is reshaped inside this frame, not the caller's.
                if let Value::Var(slot) = peel_stack_ref(arg, data)
                    && def
                        .attributes
                        .get(usize::from(*slot))
                        .is_some_and(|a| matches!(a.typedef, Type::RefVar(_)))
                {
                    forwards
                        .entry((*callee, i))
                        .or_default()
                        .push((d_nr, *slot));
                }
            }
        });
    }
    while let Some(key) = work.pop() {
        let Some(ups) = forwards.get(&key) else {
            continue;
        };
        for (caller, slot) in ups {
            if out.entry(*caller).or_default().insert(*slot) {
                work.push((*caller, *slot));
            }
        }
    }
    out
}

/// @PLN130 F2/F8 — why a view had to give up its alias.
///
/// The two causes read differently to an author and have different remedies, so the walk
/// carries which one fired. A view reached by both reports the reshape: that is the cause
/// with something to act on at the container.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewCause {
    /// The container is RESHAPED — `v.remove(i)` / `e#remove` renumbers its positions (F2).
    Reshaped,
    /// The container GROWS — an append, an insert or a keyed add can move every element to a
    /// larger record (@FR-B-Disturb's fourth event, loft#1373).
    Grown,
    /// The container VARIABLE is re-established, so the name stops meaning the store (F8).
    Reassigned,
}

/// One disturbance of a container, as the walk saw it.
///
/// The cause decides the advice line; `line` and `via` exist only so the F9 refusal can point
/// its caret at the statement responsible instead of at the whole function.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Disturbance {
    pub(super) cause: ViewCause,
    /// Source line of the statement that disturbed the container.
    pub(super) line: u32,
    /// The callee that did the removal, when it was not this frame's own statement.
    pub(super) via: Option<u32>,
    /// The container the view names — carried so a diagnostic can name it without re-deriving
    /// the view→container mapping from a frame that has since closed.
    pub(super) container: u16,
    /// A `Grown` that is the REFILL of a whole-value assignment, `v = [R { n: 5 }]`: a
    /// vector local is given its new value as a `Set` of a fresh store followed by the appends
    /// that fill it, so the disturbance a view sees is the append.  The cause stays `Grown`,
    /// because that is what decides which bindings survive; this only picks the SENTENCE —
    /// "cannot grow `v`" named an act the line does not perform.
    pub(super) replaced: bool,
}

impl Disturbance {
    /// The cause as the AUTHOR reads it: a refill after a whole-value assignment is a
    /// replacement, whatever mechanism carried it (`Disturbance::replaced`).
    pub(super) fn reported_cause(&self) -> ViewCause {
        if self.replaced {
            ViewCause::Reassigned
        } else {
            self.cause
        }
    }
}

/// @PLN164 C3 (@FR-B-Disturb, @FR-B-View) — every definition that GROWS or REMOVES FROM a
/// container reached through one of its parameters, which places, and which cause, CLOSED OVER
/// THE CALL GRAPH.
///
/// `(B-Disturb)`'s events end a place wherever they happen: `(B-Ref-Reshape)` states it
/// outright — *"the disturbance may be in this frame or in anything the frame CALLS … at any
/// depth"* — and `(B-View)` keys its materialise on the same four events, so it inherits the
/// same reach. This is the fact the materialise was missing; see [`ViewWalk::disturb`].
pub type DisturbedParams = HashMap<u32, HashMap<ParamPlace, ViewCause>>;

/// @PLN164 C5 — the places `code` DISTURBS in the frame of `def_nr`: what its own ops grow or
/// remove from, plus what anything it CALLS grows or removes from, mapped back onto the
/// arguments this frame passed.
///
/// One composer for a consumer outside the scope pass — generation's view-leaf gate asks
/// *"is the place my leaf views disturbed between this call and the last read of it?"*, which
/// is [`ViewWalk::disturb`]'s question asked over a statement span instead of a binding, and it
/// has to be answered off the same producers or the two can disagree about what a disturbance
/// is.  `disturbed` is [`disturbed_params_map`]'s answer for the whole program; `None` leaves
/// only this frame's own half, which is the pre-C3 reach and never more.
///
/// The answer is a LOWER bound in exactly the ways its producers are — a container named by
/// something other than a variable or a one-step projection is not collected — so a caller that
/// must be conservative has to treat an unresolvable place as disturbed itself.  What it never
/// does is report a disturbance that did not happen: every place here comes from a growth, a
/// removal or a re-establishment the code spells.
#[must_use]
pub fn places_disturbed_by(
    code: &Value,
    data: &Data,
    def_nr: u32,
    database: Option<&crate::database::Stores>,
    disturbed: Option<&DisturbedParams>,
) -> HashSet<ParamPlace> {
    let function = &data.def(def_nr).variables;
    let mut out = grown_containers(code, data, function, database, &HashSet::default());
    out.extend(reshaped_containers(code, data, function));
    if let Some(map) = disturbed {
        code.walk(&mut |v| {
            let Value::Call(d, args) = v else { return };
            let Some(places) = map.get(d) else { return };
            for &(slot, inner) in places.keys() {
                if let Some(arg) = args.get(usize::from(slot))
                    && let Some(base) = call_arg_place(arg, data)
                    && let Some(place) = compose_param_place(base, inner)
                {
                    out.insert(place);
                }
            }
        });
    }
    out
}

/// The parameter places `d_nr`'s OWN body disturbs — the direct answer [`disturbed_params_map`]
/// closes over the call graph.
///
/// Read through the same two producers the inline walk uses ([`grown_containers`],
/// [`reshaped_containers`]), so a callee's growth and the caller's own growth cannot be
/// answered by two different readers, and the `OpClearVector` subtraction comes with them: a
/// callee that REBUILDS the field it was handed (`sc.els = [x]` lowers to a clear and then the
/// appends) disturbs nothing, exactly as the same statement written inline disturbs nothing —
/// `(B-Disturb)` is explicit that overwriting a place is not disturbing it, and the pair was
/// measured to agree.
///
/// Only places rooted at a VISIBLE argument slot are kept. A container the callee minted itself
/// dies with the callee and no caller can hold a view into it; and the callee's own hidden
/// RETURN BUFFER is an argument slot too, so without that test every record-returning function
/// that builds a vector field reports a disturbance — `fn mk() -> Sc { Sc { els: […] } }` grows
/// `__retbuf.els`, which is the callee BUILDING its result, not a place the caller was already
/// viewing (`(O-Buffer)`: the buffer is the caller's store, and the value only becomes the
/// caller's at the bind).
fn disturbed_param_places(
    data: &Data,
    d_nr: u32,
    database: Option<&crate::database::Stores>,
) -> HashMap<ParamPlace, ViewCause> {
    let def = data.def(d_nr);
    let mut out: HashMap<ParamPlace, ViewCause> = HashMap::default();
    if def.attributes.is_empty() || matches!(def.code, Value::Null) {
        return out;
    }
    let function = &def.variables;
    // Accumulated over the WHOLE body, as the inline walk accumulates it, so a field cleared
    // anywhere is subtracted everywhere: a missed disturbance costs a materialise.
    let mut cleared: HashSet<ParamPlace> = HashSet::default();
    def.code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        if data.def(*d).name() != "OpClearVector" {
            return;
        }
        if let Some(place) = args.first().and_then(|a| base_container_place(a, data)) {
            cleared.insert(place);
        }
    });
    // A hidden attribute is COMPILER-GENERATED, which is the one test that covers the return
    // buffer and every other `__`-named slot alike; a user parameter is never one.
    let visible = |v: u16| function.is_argument(v) && !function.is_compiler_generated(v);
    for place in grown_containers(&def.code, data, function, database, &cleared) {
        if visible(place.0) {
            out.insert(place, ViewCause::Grown);
        }
    }
    // Recorded LAST and unconditionally, because a place reached by both reports the RESHAPE —
    // the cause with something to act on at the container ([`record_cause`]).
    for place in reshaped_containers(&def.code, data, function) {
        if visible(place.0) {
            out.insert(place, ViewCause::Reshaped);
        }
    }
    out
}

/// [`DisturbedParams`] for the whole program.
///
/// Closed over the call graph by the same worklist [`removed_params_map`] uses, and for the
/// same measured reason: without it the answer is one frame deep, and extracting the append
/// into a helper makes the materialise disappear and the corrupt read come back.
///
/// The forward edge is wider than that one's, because the DISTURBANCE is: a caller forwards a
/// place whenever the argument it passes is rooted at one of its OWN parameter slots, whether
/// that parameter is spelled `&` or not. A plain heap parameter aliases the caller's container
/// exactly as a `&` one does (`calls.md` F-ParamHeap, and probe 40 cell X9 measured it), so
/// keying on the spelling would let an author lose the materialise by taking loft's own
/// `warn_redundant_amp` advice.
///
/// A callee reached only through a runtime fn-ref has no static call edge and keeps today's
/// behaviour — the lower bound, in the direction that costs a materialise rather than a
/// program's meaning.
pub fn disturbed_params_map(
    data: &Data,
    database: Option<&crate::database::Stores>,
) -> DisturbedParams {
    let mut out = DisturbedParams::default();
    let trace = crate::env_once!(std::env::var_os("LOFT_TRACE_DISTURB").is_some());
    let mut work: Vec<(u32, ParamPlace)> = Vec::new();
    // (callee, its param slot) -> every (caller, the caller's own place) that feeds it.
    let mut forwards: HashMap<(u32, u16), Vec<(u32, ParamPlace)>> = HashMap::default();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !def.name.starts_with("n_") {
            continue;
        }
        // By reference: consuming the map runs `RawIntoIter::drop`, whose test of an
        // allocation-less table's `Option<(ptr, Layout, _)>` memcheck reports as a
        // conditional jump on uninitialised bytes, on every run.
        let places = disturbed_param_places(data, d_nr, database);
        for (&place, &cause) in &places {
            if trace {
                let (slot, off) = place;
                let field = if off == ANY_FIELD {
                    "whole".to_string()
                } else {
                    format!("+{off}")
                };
                eprintln!(
                    "[disturb] {} {cause:?} through parameter `{}` ({field})",
                    def.name(),
                    def.variables.name(slot)
                );
            }
            if out.entry(d_nr).or_default().insert(place, cause).is_none() {
                work.push((d_nr, place));
            }
        }
        if def.attributes.is_empty() {
            continue;
        }
        let function = &def.variables;
        def.code.walk(&mut |v| {
            let Value::Call(callee, args) = v else { return };
            if !data.def(*callee).name.starts_with("n_") {
                return;
            }
            for (i, arg) in args.iter().enumerate() {
                let Ok(i) = u16::try_from(i) else { continue };
                let Some(place) = call_arg_place(arg, data) else {
                    continue;
                };
                // Only a place rooted at the caller's OWN parameter reaches ITS caller; a
                // local container passed down is disturbed inside this frame, where the
                // inline producers already see it.
                if function.is_argument(place.0) {
                    forwards
                        .entry((*callee, i))
                        .or_default()
                        .push((d_nr, place));
                }
            }
        });
    }
    while let Some((callee, (slot, inner))) = work.pop() {
        let Some(ups) = forwards.get(&(callee, slot)) else {
            continue;
        };
        let cause = out
            .get(&callee)
            .and_then(|m| m.get(&(slot, inner)))
            .copied()
            .unwrap_or(ViewCause::Grown);
        for (caller, base) in ups.clone() {
            let Some(place) = compose_param_place(base, inner) else {
                continue;
            };
            if out
                .entry(caller)
                .or_default()
                .insert(place, cause)
                .is_none()
            {
                if trace {
                    let def = data.def(caller);
                    let (slot, off) = place;
                    let field = if off == ANY_FIELD {
                        "whole".to_string()
                    } else {
                        format!("+{off}")
                    };
                    eprintln!(
                        "[disturb] {} {cause:?} through parameter `{}` ({field}), via {}",
                        def.name(),
                        def.variables.name(slot),
                        data.def(callee).name()
                    );
                }
                work.push((caller, place));
            }
        }
    }
    out
}
