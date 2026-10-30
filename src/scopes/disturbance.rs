// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! What code DISTURBS: the containers a body grows, removes from or reshapes, directly or through
//! a `&` parameter of a callee, closed over the call graph — and the sentence that tells the author
//! why a view had to be copied out of its container.

use super::places::{
    ParamPlace, add_offset, base_container_place, call_arg_at, compose_param_at, named_place_depth,
    peel_stack_ref,
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
        ViewCause::Released => {
            crate::copy_manifest::note_released_view(vname, cname, fname, via);
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
/// A container is named through [`named_place_depth`] — a variable, a field run, or a chain
/// through elements, each with its depth — and a container reached any other way (a call
/// result) is not recognised: the answer is a lower bound there, and a missed case keeps
/// today's behaviour rather than inventing a new one.
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
) -> HashMap<(u16, u32), ContainerAt> {
    let mut out = places_named_by(code, data, &|name| match name {
        "OpRemove" => Some(1),
        "OpRemoveVector" | "OpKeepVectorRange" | "OpPrependVector" => Some(0),
        _ => None,
    });
    for (place, at) in released_children(code, data) {
        let e = out.entry(place).or_insert(at);
        e.depth = e.depth.min(at.depth);
        e.index = None;
    }
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
    for (place, at) in places_named_by(code, data, &|name| match name {
        "OpHashRemove" => Some(0),
        _ => None,
    }) {
        if place.1 != ANY_FIELD || at.depth != 0 {
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
            out.insert(place, at);
        }
    }
    out
}

/// The out-of-line FIELDS `code` writes a value that may be null — `@FR-B-OutOfLine`: such a
/// write releases the child, which ends every view of it.  A record literal is present, and a
/// present value overwrites the child in place, where a view keeps reading it.  Part of
/// [`reshaped_containers`]; asked apart only for the advice, which names the null write.
pub(super) fn released_children(code: &Value, data: &Data) -> HashMap<(u16, u32), ContainerAt> {
    let mut out: HashMap<(u16, u32), ContainerAt> = HashMap::default();
    code.walk(&mut |v| {
        if let Value::Call(d, args) = v
            && data.def(*d).name() == "OpSetChildRec"
            && let [field, value, _] = &args[..]
            && !matches!(value.unspan(), Value::Block(_) | Value::Insert(_))
            && let Some((place, depth)) = named_place_depth(field, data)
        {
            let at = out
                .entry(place)
                .or_insert(ContainerAt { depth, index: None });
            at.depth = at.depth.min(depth);
        }
    });
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
///
/// Each place comes with its container's DEPTH — the element reads between it and its root
/// variable ([`named_place_depth`]) — because a growth ends only the places INSIDE the
/// container it grows: `vv[0] += [x]` is a depth-1 container, so it ends `vv[0][0]` (depth 2)
/// and leaves `a = vv[0]` (depth 1, the element SLOT) re-reading the repointed handle.  A place
/// grown at two depths keeps the shallower one, which ends more.
///
/// A REBUILT field (`w.a = [..]`, an `OpClearVector` and then these same appends) is not
/// subtracted here: what a rebuild spares is the view OF the field, not the views into its
/// elements, and only the caller holds the views to tell the two apart.
pub(super) fn grown_containers(
    code: &Value,
    data: &Data,
    function: &Function,
    database: Option<&crate::database::Stores>,
) -> HashMap<(u16, u32), ContainerAt> {
    let mut out: HashMap<(u16, u32), ContainerAt> = HashMap::default();
    let mut add = |place: (u16, u32), depth: u8, index: Option<(u32, i32)>| {
        let g = out.entry(place).or_insert(ContainerAt { depth, index });
        g.depth = g.depth.min(depth);
        if g.index != index {
            g.index = None;
        }
    };
    code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let name = data.def(*d).name();
        // @PLN157 § V-m — a fused scalar append (`OpPush<Kind>`) grows its container, named at
        // arg 0 as every spelling below names it.
        let fused = name.starts_with("OpPush");
        if !fused
            && !matches!(
                name,
                "OpNewRecord"
                    | "OpPreAllocVector"
                    | "OpAppendVector"
                    | "OpInsertVector"
                    | "OpHashAdd"
            )
        {
            return;
        }
        // The container is named through the ONE resolver the view side uses, whatever its
        // spelling: a variable (`v += …`), a field (`w.a += …`), a field of a field
        // (`o.inner.a += …`) or a container inside an element (`vv[0] += …`).  Each spelling
        // matched its own shape here before, and only `OpNewRecord` and the fused push took a
        // field at all — so `w.a += other`, `w.a.insert(…)`, `o.inner.a += [x]` and
        // `vv[0] += [x]` moved the elements under a live view and the view read the freed
        // block, on both backends and in silence (loft#2161).
        let Some(arg) = args.first() else { return };
        let Some((parent, depth)) = named_place_depth(arg, data) else {
            return;
        };
        let index = crate::use_analysis::root_element_index(data, arg);
        if fused || name != "OpNewRecord" {
            add(parent, depth, index);
            return;
        }
        // `OpNewRecord(parent, tp, fld)` names its container in TWO parts: `fld == u16::MAX`
        // is an append to `parent` itself (`v += [x]`), any other `fld` an append to that
        // FIELD of it (`w.a += [x]` emits `OpNewRecord(w, tp, 1)`).  The field is a NUMBER and
        // a place carries a byte OFFSET, so it is converted — `Stores::field_position` on
        // `tp`, the parent's own struct type — and added to the parent's offset.
        //
        // Without the store the conversion cannot be made, and the field-qualified growth is
        // left UNCOLLECTED rather than widened to the parent: reading the parent alone shakes
        // every view rooted at it whichever field it names, which silently emptied
        // `moros_editor`'s undo stack when loft#1373 first shipped.
        let fld = match args.get(2).map(Value::unspan) {
            Some(Value::Int(f)) => *f,
            _ => return,
        };
        if fld == i32::from(u16::MAX) {
            add(parent, depth, index);
            return;
        }
        let Some(db) = database else { return };
        let tp = match (arg.unspan(), args.get(1).map(Value::unspan)) {
            (Value::Var(c), _) => {
                let def = data.type_def_nr(function.tp(*c).base());
                if def == u32::MAX {
                    return;
                }
                data.def(def).known_type()
            }
            (_, Some(Value::Int(tp))) => match u16::try_from(*tp) {
                Ok(tp) => tp,
                Err(_) => return,
            },
            _ => return,
        };
        let off = db.field_position(tp, fld as u16);
        if off == u16::MAX {
            return;
        }
        add(
            (parent.0, add_offset(parent.1, u32::from(off))),
            depth,
            index,
        );
    });
    out
}

/// Where a growth or a removal reached its container: the element reads between the container
/// and its root ([`named_place_depth`]), and the literal index of the one nearest the root when
/// every event at that place used the same one ([`crate::use_analysis::root_element_index`]) —
/// the proof a view of a DIFFERENT element is spared on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContainerAt {
    pub depth: u8,
    pub index: Option<(u32, i32)>,
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
/// [`value_view_place`] resolves a projection chain to its place — so only this half was short
/// and the two never met.
///
/// A container reached through a CALL result is still uncollected, and that stays the lower
/// bound it always was.
///
/// Each place comes with where its container sits ([`ContainerAt`]) — `vv[0].remove(0)` is a
/// container one element read deep, which ends `vv[0][1]` and leaves `a = vv[1]` and the slot
/// view `a = vv[0]` alone, the answer a growth of the same container gives (loft#2161).
fn places_named_by(
    code: &Value,
    data: &Data,
    which: &dyn Fn(&str) -> Option<usize>,
) -> HashMap<(u16, u32), ContainerAt> {
    let mut out: HashMap<(u16, u32), ContainerAt> = HashMap::default();
    code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let Some(at) = which(data.def(*d).name()) else {
            return;
        };
        let Some(arg) = args.get(at) else { return };
        if let Some((place, depth)) = named_place_depth(arg, data) {
            let index = crate::use_analysis::root_element_index(data, arg);
            let c = out.entry(place).or_insert(ContainerAt { depth, index });
            c.depth = c.depth.min(depth);
            if c.index != index {
                c.index = None;
            }
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
            "OpRemoveVector" | "OpKeepVectorRange" | "OpPrependVector" => args.first(),
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
    /// An out-of-line field is written a value that may be null, which releases the child a
    /// view names (`@FR-B-OutOfLine`).
    Released,
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
pub type DisturbedParams = HashMap<u32, HashMap<ParamPlace, ParamDisturbance>>;

/// What a definition does to one place of a parameter: the cause, and where the container
/// sits inside the parameter ([`ContainerAt`]) — `p[0].remove(0)` is a container one element
/// read deep, which ends the caller's views INTO that element and not its view of the element
/// slot, exactly as the same statement written in the caller does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamDisturbance {
    pub cause: ViewCause,
    pub at: ContainerAt,
}

impl ParamDisturbance {
    /// Fold a second disturbance of the same place in, answering whether anything changed: the
    /// shallower container (it ends more), the literal index only where both agree, and the
    /// reshape over the growth ([`record_cause`]).  Each moves one way only, which is what
    /// ends [`disturbed_params_map`]'s worklist.
    fn merge(&mut self, other: ParamDisturbance) -> bool {
        let before = *self;
        self.at.depth = self.at.depth.min(other.at.depth);
        if self.at.index != other.at.index {
            self.at.index = None;
        }
        if other.cause == ViewCause::Reshaped {
            self.cause = ViewCause::Reshaped;
        }
        *self != before
    }
}

/// Record `d` at `place`, merged with what is already there; whether the map changed.
fn record_param(
    out: &mut HashMap<ParamPlace, ParamDisturbance>,
    place: ParamPlace,
    d: ParamDisturbance,
) -> bool {
    match out.entry(place) {
        std::collections::hash_map::Entry::Vacant(e) => {
            e.insert(d);
            true
        }
        std::collections::hash_map::Entry::Occupied(mut e) => e.get_mut().merge(d),
    }
}

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
/// something other than a variable or a projection chain is not collected — so a caller that
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
    // Every depth, as the root place: this consumer must be conservative, and a growth of a
    // container inside an element is still a disturbance of a place rooted where it is.
    let mut out: HashSet<ParamPlace> = grown_containers(code, data, function, database)
        .into_keys()
        .collect();
    out.extend(reshaped_containers(code, data, function).into_keys());
    if let Some(map) = disturbed {
        code.walk(&mut |v| {
            let Value::Call(d, args) = v else { return };
            let Some(places) = map.get(d) else { return };
            for (&(slot, inner), pd) in places {
                if let Some(arg) = args.get(usize::from(slot))
                    && let Some((base, base_at)) = call_arg_at(arg, data)
                {
                    out.insert(compose_param_at(base, base_at, inner, pd.at).0);
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
/// answered by two different readers.  A callee that REBUILDS the field it was handed
/// (`sc.els = [x]` lowers to a clear and then the appends) ends the caller's views into its
/// elements and spares its view of the field, exactly as the same statement written inline
/// does — the caller's walk spares every view at the container's own depth.
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
) -> HashMap<ParamPlace, ParamDisturbance> {
    let def = data.def(d_nr);
    let mut out: HashMap<ParamPlace, ParamDisturbance> = HashMap::default();
    if def.attributes.is_empty() || matches!(def.code, Value::Null) {
        return out;
    }
    let function = &def.variables;
    // A REBUILD of a field the callee was handed (`p.els = [x]`: a clear, then the appends)
    // ends the caller's views into its elements and not its view of the field, which is the
    // depth rule the caller's walk applies to every event read off this map — so the clear is
    // reported as what it does to those elements, a removal of all of them.  Subtracted as an
    // overwrite before, which spared the views into the elements too (loft#2161).
    let mut cleared: HashMap<ParamPlace, ParamDisturbance> = HashMap::default();
    def.code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        if data.def(*d).name() != "OpClearVector" {
            return;
        }
        let Some(arg) = args.first() else { return };
        if let Some((place, depth)) = named_place_depth(arg, data) {
            let index = crate::use_analysis::root_element_index(data, arg);
            record_param(
                &mut cleared,
                place,
                ParamDisturbance {
                    cause: ViewCause::Reshaped,
                    at: ContainerAt { depth, index },
                },
            );
        }
    });
    // A hidden attribute is COMPILER-GENERATED, which is the one test that covers the return
    // buffer and every other `__`-named slot alike; a user parameter is never one.
    let visible = |v: u16| function.is_argument(v) && !function.is_compiler_generated(v);
    // Every depth, each with where its container sits: the caller composes it with the depth
    // of the argument it passed ([`compose_param_at`]) and ends the views the same statement
    // written inline would end.  Kept to depth 0 before, which dropped `p[0].remove(0)` — a
    // removal `main` had reported as one of the whole parameter — and the caller's view of
    // the element that shifted in read the wrong record (loft#2161).
    for (place, at) in grown_containers(&def.code, data, function, database) {
        if visible(place.0) {
            let cause = ViewCause::Grown;
            record_param(&mut out, place, ParamDisturbance { cause, at });
        }
    }
    // A place reached by both reports the RESHAPE — the cause with something to act on at the
    // container ([`record_cause`]); [`ParamDisturbance::merge`] keeps it.
    for (place, at) in reshaped_containers(&def.code, data, function) {
        if visible(place.0) {
            let cause = ViewCause::Reshaped;
            record_param(&mut out, place, ParamDisturbance { cause, at });
        }
    }
    for (place, d) in cleared {
        if visible(place.0) {
            record_param(&mut out, place, d);
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
    let say = |d_nr: u32, (slot, off): ParamPlace, d: ParamDisturbance, via: Option<u32>| {
        let def = data.def(d_nr);
        let field = if off == ANY_FIELD {
            "whole".to_string()
        } else {
            format!("+{off}")
        };
        let via = via.map_or(String::new(), |c| format!(", via {}", data.def(c).name()));
        eprintln!(
            "[disturb] {} {:?} through parameter `{}` ({field}, depth {}){via}",
            def.name(),
            d.cause,
            def.variables.name(slot),
            d.at.depth
        );
    };
    let mut work: Vec<(u32, ParamPlace)> = Vec::new();
    // (callee, its param slot) -> every (caller, the caller's own place, where it sits) that
    // feeds it.
    type Forward = (u32, ParamPlace, ContainerAt);
    let mut forwards: HashMap<(u32, u16), Vec<Forward>> = HashMap::default();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !def.name.starts_with("n_") {
            continue;
        }
        // By reference: consuming the map runs `RawIntoIter::drop`, whose test of an
        // allocation-less table's `Option<(ptr, Layout, _)>` memcheck reports as a
        // conditional jump on uninitialised bytes, on every run.
        let places = disturbed_param_places(data, d_nr, database);
        for (&place, &d) in &places {
            if trace {
                say(d_nr, place, d, None);
            }
            if record_param(out.entry(d_nr).or_default(), place, d) {
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
                let Some((place, at)) = call_arg_at(arg, data) else {
                    continue;
                };
                // Only a place rooted at the caller's OWN parameter reaches ITS caller; a
                // local container passed down is disturbed inside this frame, where the
                // inline producers already see it.
                if function.is_argument(place.0) {
                    forwards
                        .entry((*callee, i))
                        .or_default()
                        .push((d_nr, place, at));
                }
            }
        });
    }
    while let Some((callee, (slot, inner))) = work.pop() {
        let Some(ups) = forwards.get(&(callee, slot)) else {
            continue;
        };
        let Some(&d) = out.get(&callee).and_then(|m| m.get(&(slot, inner))) else {
            continue;
        };
        for &(caller, base, base_at) in ups {
            let (place, at) = compose_param_at(base, base_at, inner, d.at);
            let up = ParamDisturbance { cause: d.cause, at };
            if record_param(out.entry(caller).or_default(), place, up) {
                if trace {
                    say(caller, place, up, Some(callee));
                }
                work.push((caller, place));
            }
        }
    }
    out
}
