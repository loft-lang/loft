// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! The program shapes the compiler REFUSES because a callee reshapes a container an argument
//! names an element of — found per definition and reported with the line they were written on.

use super::disturbance::{
    DisturbedParams, RemovedParams, ViewCause, disturbed_params_map, removed_params_map,
};
use super::places::{
    ParamPlace, call_arg_place, compose_param_place, same_place, value_view_places,
};
use super::view_walk::{ViewWalk, is_place_link};
use crate::data::{Data, DefType, Type, Value};
use crate::fxhash::FxHashSet as HashSet;
use crate::use_analysis::ANY_FIELD;
use crate::variables::Function;

/// Visit every node of `code`, telling `f` which source LINE is in effect for it.
///
/// A statement's line is not on the statement: a block interleaves `Value::Line(n)` markers
/// with its operators, so the line has to be carried down the walk. `Block`, `Loop` and
/// `Insert` therefore re-read it per operator; every other form inherits its parent's.
fn walk_lined(code: &Value, line: u32, f: &mut impl FnMut(&Value, u32)) {
    let node = code.unspan();
    let stmts: Option<&[Value]> = match node {
        Value::Block(b) | Value::Loop(b) => Some(&b.operators),
        Value::Insert(ops) => Some(ops),
        _ => None,
    };
    f(node, line);
    if let Some(stmts) = stmts {
        let mut cur = line;
        for s in stmts {
            if let Value::Line(n) = s.unspan() {
                cur = *n;
            }
            walk_lined(s, cur, f);
        }
    } else {
        node.for_each_child(&mut |c| walk_lined(c, line, f));
    }
}

/// @PLN130 F9 (@FR-B-Ref-Reshape) — does argument `arg` name an ELEMENT inside `place`, the
/// container the callee disturbs?
///
/// Every way an author can write it, because they reach the check in different shapes:
///
/// - **bound earlier** (`t = v[2]; f(t, v)`) — `t` arrives as a plain `Var` and carries `v` in
///   its type deps, which is the borrow relation itself.  For a whole container that is the
///   answer.  For a container in a FIELD the deps name only the variable, so the local's own
///   bindings say which field: one that resolves to another field names nothing the callee
///   moves, and one that does not resolve is taken to name it — refusing is the direction that
///   costs a program a rewrite, where admitting costs it its meaning;
/// - **written into the call** (`f(v[2], v)`) — the parser does not leave that inline. It lifts
///   the projection into a temp first, so the argument is
///   `Insert([Set(t, OpGetVector(v, …)), OpCreateStack(t)])` and the alias is a `Set` INSIDE the
///   argument expression. Reading only the argument's value misses it, which is how loft#779's
///   own repro (`shift(v[2], v)`) went unreported while `t = v[2]; shift(t, v)` did not;
/// - **not a variable at all** — a `?`-discharged element is a block whose tail names the temp
///   the block bound, and the NULLABLE element read a format string passes
///   (`OpGetVectorNullable`) is a second spelling of the same projection.  Both are answered by
///   [`names_element_in`] over the WHOLE argument, not its tail (loft#1554).
///
/// The lifted `Set` is looked up by the temp the argument actually passes rather than by
/// searching the expression for any projection of the container: `f(w[v[0].n], v)` mentions
/// `v[0]` but passes an element of `w`, and a search would refuse it.
fn arg_references_element_in(
    arg: &Value,
    place: ParamPlace,
    body: &Value,
    function: &Function,
    data: &Data,
) -> bool {
    let Value::Var(t) = arg_target(arg, data) else {
        let whole = match arg.unspan() {
            Value::Call(cs, cargs) if data.def(*cs).name() == "OpCreateStack" => {
                cargs.first().unwrap_or(arg)
            }
            _ => arg,
        };
        return names_element_in(whole, place, function, data);
    };
    if *t == place.0 {
        return false;
    }
    // A lifting preamble inside the argument binds the temp it passes.
    let mut lifted = false;
    arg.walk(&mut |n| {
        if let Value::Set(s, rhs) = n
            && *s == *t
            && names_element_in(rhs, place, function, data)
        {
            lifted = true;
        }
    });
    if lifted {
        return true;
    }
    if !function.tp(*t).depend().contains(&place.0) {
        return false;
    }
    if place.1 == ANY_FIELD {
        return true;
    }
    let mut resolved = true;
    let mut names = false;
    body.walk(&mut |n| {
        let Value::Set(s, rhs) = n else { return };
        if *s != *t || matches!(rhs.unspan(), Value::Null) {
            return;
        }
        let places = value_view_places(rhs, data, function);
        if places.is_empty() {
            resolved = false;
        }
        names |= places.iter().any(|p| same_place(*p, place));
    });
    names || !resolved
}

/// Does `value` name an ELEMENT inside `place` — a read that crossed an element of the
/// container there, rather than the container's own slot (`&cv.data`, which a growth repoints
/// and the reader re-reads, `(B-Ref-Alias)`'s in-versus-to)?
///
/// Through [`value_view_places`], the one home for "which places does this value view", so a
/// `?`-discharged or a nullable element read is the projection it is.  A value that names no
/// place answers no: a scalar, a fresh record and a call's result alias nothing the callee
/// can move.
fn names_element_in(value: &Value, place: ParamPlace, function: &Function, data: &Data) -> bool {
    if matches!(
        crate::use_analysis::view_source_place_indexed(data, value),
        Some((_, false))
    ) {
        return false;
    }
    value_view_places(value, data, function)
        .iter()
        .any(|p| same_place(*p, place))
}

/// The value an argument ultimately passes: the tail of any lifting preamble, with the
/// `OpCreateStack` wrapper a `&` parameter adds peeled off.
///
/// Deliberately NOT folded into [`peel_stack_ref`]: that one answers which variable a `&`
/// CONTAINER argument names, where no lift is involved, and widening it would quietly change
/// what @PLN130 F8's `established_stores` treats as a reassignment.
fn arg_target<'a>(arg: &'a Value, data: &Data) -> &'a Value {
    let inner = arg.tail().unspan();
    if let Value::Call(cs, cargs) = inner
        && data.def(*cs).name() == "OpCreateStack"
        && let Some(first) = cargs.first()
    {
        return first.tail().unspan();
    }
    inner
}

/// @PLN130 F9 — one program shape the compiler REFUSES, ready to be reported.
///
/// Carries a position rather than being emitted here, because the analysis runs over `Data`
/// (where every callee's body is available) while the diagnostics collector lives on the
/// parser's lexer.
pub struct ReshapeRefusal {
    pub file: String,
    pub line: u32,
    pub message: String,
}

/// @PLN130 F9 / [loft#779](https://github.com/loft-lang/loft/issues/779) — the shapes where a
/// container is reshaped while a reference into it is still live, which loft REFUSES.
///
/// **B-Ref-Alias is unconditional** — a `&` binding is a live link to the source, so every
/// write through it reaches the source. There is exactly one program shape where it cannot:
/// `remove` renumbers the positions inside a container's store, and a reference is pinned to
/// one, so a write through it lands on the wrong element or on a vacated slot and is lost.
/// Rather than carry runtime machinery to re-point the link, that shape is rejected before it
/// runs (maker, 2026-08-05). It is the rustc bargain in loft's spelling: where rustc refuses
/// the mutation while a borrow is live, loft refuses the removal.
///
/// Two producers, because a reference into a container reaches a removal two ways:
///
/// 1. a **`&` LINK in this frame** (`c = &v[0]`) that is live across a removal from `v` — the
///    removal being either this frame's own or one a callee does through a `&` parameter;
/// 2. a **CALL that is handed both a container and a reference into it** (`shift(v[2], v)`),
///    where the callee removes from the container parameter. Checked at the CALL SITE, which is
///    the only place the two arguments are known to name the same store: inside the callee they
///    are two unrelated parameters, and refusing there would reject sound programs.
///
/// **Liveness is the condition, not existence** — the rustc rule, and the same walk F2 uses.
/// `c = &v[0]; c.n = 1; v.remove(0);` keeps compiling: the link is dead before the removal, so
/// there is no conflict and the write lands.
///
/// Producer 2 does **not** ask whether the argument was spelled `&`, and that is measured, not
/// an oversight: a plain struct parameter aliases the caller's element exactly as a `&` one
/// does (`fn w(t: Box) { t.n = 99 }` called as `w(v[2])` writes 99 into `v` — and loft's own
/// `warn_redundant_amp` advice tells authors so). Refusing only the `&` spelling would mean an
/// author who takes that advice and drops the `&` trades a compile error for a silent lost
/// write. Producer 1 is `&`-only for the opposite and equally measured reason: a PLAIN local
/// bind does not alias across a reshape, because @PLN130 F2 materialises it and says so.
///
/// Known lower bound, in the safe direction (the refusal simply does not fire): a callee
/// reached only through a runtime fn-ref has no static call edge, so [`removed_params_map`]'s
/// closure cannot follow it. Declaration order does NOT matter — the check runs once the whole
/// world is parsed, so a callee written below its caller is answered the same way.
///
/// Every definition is checked, the stdlib's included. Filtering by `source` was tried and is
/// wrong: `Parser::parse_str` — the whole Rust test harness — never leaves `STD_SOURCE`, so the
/// filter silently made the check a no-op there while it still fired on a file. A pass over
/// definitions that cannot possibly trip it is the cheaper mistake.
#[must_use]
pub fn reshape_refusals(data: &Data, database: &crate::database::Stores) -> Vec<ReshapeRefusal> {
    let removed = removed_params_map(data);
    // @FR-B-Ref-Reshape — *"The disturbance may be in this frame or in anything the frame
    // CALLS"*, and `(B-Disturb)` states the same for all four events: *"an event disturbs
    // WHEREVER IT HAPPENS — in this frame, or in anything the frame CALLS, at any depth."*
    //
    // `removed` carries only a REMOVAL, and only one spelled against the `&` parameter itself
    // (`removed_ref_params` keys on `OpRemoveVector`/`OpRemove` over a bare `Var` typed
    // `RefVar`).  `disturbed` carries the rest of that reach — a callee's GROWTH, and a removal
    // from a FIELD of a parameter — so the refusal answers the question the MATERIALISE walk
    // answers.  The case the rule states in as many words has every part spelled `&`, with the
    // callee disturbing the very parameter it was handed: `fn vgrow(v: &vector<H>, n) { v +=
    // [mk(n)] }` under a live `e = &v[0]`, which without both halves compiles and releases one
    // resource TWICE on both backends.
    //
    // Built here rather than per definition because the question is asked once per CALL and a
    // callee body would otherwise be re-walked once per call to it — the same reason
    // `removed_params_map` is built here, and the same construction the scope pass uses.
    //
    // Gated on the SAME `callee_disturb_enabled` switch as the scope pass, deliberately: the
    // switch names the callee half of `(B-Disturb)`, and that half is ONE rule's reach with two
    // consumers, not two behaviours that happen to share a cause.  The consequence is worth
    // stating, because it costs something: `LOFT_NO_CALLEE_DISTURB=1` withholds the callee half
    // from BOTH consumers at once, so it is not an A/B for the materialise alone.  That is the
    // honest meaning of the flag rather than a limitation of it — and the two halves stay
    // distinguishable at the symptom, since a refusal is loud where a materialise is quiet.
    let disturbed =
        crate::keys::callee_disturb_enabled().then(|| disturbed_params_map(data, Some(database)));
    let disturbed = disturbed.as_ref();
    let mut out: Vec<ReshapeRefusal> = Vec::new();
    for d_nr in 0..data.definitions() {
        out.extend(def_reshape_refusals(
            data, d_nr, &removed, disturbed, database,
        ));
    }
    out
}

#[expect(clippy::too_many_lines, reason = "inherited")]
fn def_reshape_refusals(
    data: &Data,
    d_nr: u32,
    removed: &RemovedParams,
    disturbed: Option<&DisturbedParams>,
    database: &crate::database::Stores,
) -> Vec<ReshapeRefusal> {
    let def = data.def(d_nr);
    if !matches!(def.def_type, DefType::Function) || matches!(def.code, Value::Null) {
        return Vec::new();
    }
    let function = &def.variables;
    let file = def.position.file;
    let mut out: Vec<ReshapeRefusal> = Vec::new();
    // (1) — a `&` link this frame holds, still live where its container is disturbed. Every
    // cause the walk reports is refused: each one ends the place the reference names, and a
    // reference that cannot reach its source is not what `&` asked for.  That includes the
    // GROWTH the walk learned in loft#1373 — `c = &v[0]; v += [x]; c.n` names an element the
    // growth may have moved, which is the same reason the other three are refused.
    //
    // @FR-B-Ref-Reshape — the walk is handed the STORE, because a growth names its container
    // by field NUMBER (`OpNewRecord(b, tp, 1)`) while a view carries a byte OFFSET, and
    // `Stores::field_position` is the only thing that converts between them.  Without it
    // `grown_containers` leaves every field-qualified growth UNCOLLECTED, so
    // `c = &b.v[0]; b.v += [x]` goes unrefused while the same growth of a plain LOCAL is
    // refused, and a removal from the same field is: the rule's answer would depend on where
    // the container is stored.  The materialise walk has the store on every path, which is why
    // that side copies the link (and tells the author) wherever this side would say nothing.
    // The callee's half (`disturbed`) is handed to the REFUSAL as well as the materialise.  It
    // needs no `&` case of its own: `shake_places_keyed` spares a view only when the keyed
    // filter proves a different record or when `names_container_itself` says the binding names
    // the CONTAINER rather than a place inside it — never because it is spelled `&`.  So a link
    // INTO a disturbed container is shaken here whatever it is spelled, and this walk's answer
    // is what the refusal consumes.  A link TO one (`d = &cv.data`) stays spared, which keeps
    // `157-view-header`'s `grown_between` reading 11 — `(B-Ref-Alias)`'s in-versus-to
    // distinction, closed as D-bind-46.
    for (view, d) in ViewWalk::run(
        &def.code,
        function,
        data,
        Some(removed),
        disturbed,
        Some(database),
        def.position.line,
    ) {
        // @FR-H-View-Drop — two populations, one walk.  An `&` link asked for a reference and
        // must get one or the program is refused.  A PLAIN view of a member that owns a
        // droppable is refused for a different reason, and one the author cannot escape by
        // dropping the `&`: `(B-View)` would hand it a COPY, and a copy of a droppable is a
        // second structure holding one resource, which `(H-Lease)` does not allow.  Every other
        // type keeps the materialise-and-tell answer `(B-View)` gives it.
        //
        // No further type test belongs here, because the walk's own answer is the rest of the
        // gate: `record_target` admits a binding only when it is a view at all (`Reference |
        // Enum | Vector`, and not an iteration source) and only when its right-hand side names
        // a container.  So a TUPLE-element view — which the emitter never copies, and which
        // releases once today — is not in this set to begin with, and refusing it would reject
        // a sound program.  Measured over the cell matrix: the walk's answer and the copy-out
        // advice agree on every cell.
        // Three spellings of one question — *"did the author write `&` here?"* — and until
        // 2026-09-24 the refusal knew two of them.  `is_amp_link` is the struct projection the
        // parser leaves unlowered; `is_place_link` is the scalar or text place, lowered to a
        // `RefVar` local (D-bind-56, which merged that third reader in a day earlier).  The
        // COLLECTION link (`p = &o.v`, `a = &s.h`) is the fourth, marked
        // `is_amp_container_link` for the walk two thousand lines up — and it reached neither
        // test, so `p = &o.v; o = S { … }; p += [7]` was MATERIALISED with an advice where the
        // record and scalar spellings of the same program are refused.  `(B-Ref-Reshape)` is
        // explicit that this is the one thing a `&` may not get: *"loft will not quietly
        // downgrade the reference to a copy"*, and the parser's own note beside
        // `amp_container_link` had already written the question down as open and named the
        // rules' answer.  A PLAIN collection bind off a borrowed base aliases identically and
        // is NOT in this set: `(B-View)` says that one materialises, which is why the marker
        // exists at all.
        let amp = function.is_amp_link(view)
            || is_place_link(function, view)
            || function.is_amp_container_link(view);
        let drops = data.type_owns_droppable_anywhere(function.tp(view));
        if !amp && !drops {
            continue;
        }
        let view_name = function.name(view);
        let container = function.name(d.container);
        // A removal destroys the place for a DIFFERENT reason depending on the kind, and
        // this split only became necessary WITH loft#1460 — before it, only a `sorted`
        // reached this refusal and "renumbers" was right for the one kind that could see
        // it.  Now the record-per-element kinds reach it too, and for them nothing
        // renumbers: `(Col-RemoveKeyed)` says so, and a reader handed the vector reason
        // could check it and find it false.  What ends their place is that the removal
        // FREES a record, which a later insert may reuse.
        //
        // ⚠ loft#1458 was filed on the wording BEFORE the set widened and closed as
        // invalid, correctly — the same wording is wrong in the other direction now.  Which
        // way it is wrong depends on which kinds reach the site, so the two must move
        // together.
        //
        // `peel_link`, because the container may be a `&` one or a `τ?` one: the question is
        // what it IS, not how it is spelled or reached.
        let record_per_element = matches!(
            crate::data::Type::peel_link(function.tp(d.container)),
            Type::Hash(_, _, _) | Type::Index(_, _, _) | Type::Radix(_, _, _) | Type::Trie(_, _, _)
        );
        // The two causes destroy the place differently, so they read differently and have
        // different ways out — but the verdict is the same.
        let (what, why) = match d.cause {
            ViewCause::Reshaped if record_per_element => (
                format!("remove from `{container}`"),
                format!(
                    "a removal frees the record its key names, and a later insert can reuse \
                     it, so a write through `{view_name}` may land on a different element \
                     than the one it names"
                ),
            ),
            ViewCause::Reshaped => (
                format!("remove from `{container}`"),
                format!(
                    "a removal renumbers the remaining elements, so a write through \
                     `{view_name}` would no longer reach the element it names"
                ),
            ),
            ViewCause::Grown if d.replaced => (
                format!("give `{container}` a new value"),
                format!(
                    "`{view_name}` names an element of the value `{container}` held before, \
                     so a write through `{view_name}` would no longer reach `{container}`"
                ),
            ),
            ViewCause::Grown => (
                format!("grow `{container}`"),
                format!(
                    "a container that outgrows its allocation moves every element, so a \
                     write through `{view_name}` would no longer reach the element it names"
                ),
            ),
            ViewCause::Reassigned => (
                format!("give `{container}` a new value"),
                format!(
                    "`{view_name}` names a place inside `{container}`, and replacing \
                     `{container}` leaves that place with nothing to point at"
                ),
            ),
        };
        // The REASON differs with the population, not only the way out.  The clause above
        // explains a LOST WRITE through a `&` link — true for a reference, and beside the point
        // for a plain view, which never wrote through to its container in the first place
        // `(B-View)`.  What is wrong for this population is the COPY itself: `(H-View-Drop)`
        // keeps a view of a droppable a view, because the copy `(B-View)` would otherwise hand
        // it is a second structure holding one resource.  A reader given the other population's
        // reason could check it and find it false, which is the `(Col-RemoveKeyed)` mistake
        // loft#1458 already paid for once.
        let why = if amp {
            why
        } else {
            format!(
                "`{view_name}` would be given its own copy of `{tp}`, and a copy of a value that \
                 owns a resource is a second structure releasing that resource a second time",
                tp = data.display_type_name(function.tp(view))
            )
        };
        // The way out differs too, and the `&` one is WRONG here: "bind without `&` to work on a
        // copy" names exactly the copy `(H-Copy-Refuse)` rejects, so offering it would send the
        // author from a refused program to one that releases a resource twice.
        let cure = if amp {
            "or bind without `&` to work on a copy".to_string()
        } else {
            format!(
                "or read `{tp}` where it lives",
                tp = data.display_type_name(function.tp(view))
            )
        };
        // The CALLEE form names the callee's act before the reason, and the JOINER between them
        // differs with the population because the two clauses stand in different relations.  For
        // a `&` link they are parallel facts about the container — *"would grow `v`, AND a
        // container that outgrows its allocation moves every element"*.  For a plain droppable
        // view the growth CAUSES the copy — *"would grow `b`, SO `e` would be given its own copy
        // of `H`"*.  With one joiner for both, that population read *"would grow `b`, and `e`
        // would be given its own copy of `H`, and a copy of a value…"*: two `and`s in one
        // sentence, because this `why` opens with a clause of its own where the `&` one opens
        // with a continuation.  The inline form needs no joiner — it reaches `why` straight off
        // the dash — which is why the run-on appears only once the disturbance is a call.
        let joiner = if amp { "and" } else { "so" };
        let message = match d.via {
            Some(callee) => format!(
                "cannot call `{callee_name}` while `{view_name}` references a place inside \
                 `{container}` — `{callee_name}` would {what}, {joiner} {why}. Move the call \
                 after the last use of `{view_name}`, {cure}",
                callee_name = data.def(callee).original_name()
            ),
            None => format!(
                "cannot {what} while `{view_name}` references a place inside it — {why}. Move \
                 it after the last use of `{view_name}`, {cure}"
            ),
        };
        out.push(ReshapeRefusal {
            file: file.to_string(),
            line: d.line,
            message,
        });
    }
    // (2) — a call handed both a container and a reference into it.
    //
    // The callee's half is `disturbed` (@PLN164 C3's fact): every place it grows or removes
    // from through a parameter — a plain or a `&` one, a field inside one, at any depth.  This
    // half read `removed` alone until loft#1554, which carries one spelling of one event (a
    // removal through a bare `&vector` parameter), so a plain vector, a struct field and every
    // GROWTH compiled and read the element that moved.  `removed` is still unioned in: it is
    // what `LOFT_NO_CALLEE_DISTURB=1` leaves, and a removal both facts carry is one place.
    // The places are visited in a fixed order so the report does not depend on hashing, and a
    // removal is reported where one place is both, because a renumbering is the one the author
    // can act on at the container.
    walk_lined(&def.code, def.position.line, &mut |node, line| {
        let Value::Call(callee, args) = node else {
            return;
        };
        let cdef = data.def(*callee);
        let mut places: Vec<(ParamPlace, ViewCause)> = disturbed
            .and_then(|d| d.get(callee))
            .map(|m| m.iter().map(|(p, c)| (*p, *c)).collect())
            .unwrap_or_default();
        if let Some(params) = removed.get(callee) {
            places.extend(
                params
                    .iter()
                    .map(|k| ((*k, ANY_FIELD), ViewCause::Reshaped)),
            );
        }
        if places.is_empty() {
            return;
        }
        places.sort_by_key(|((slot, inner), cause)| {
            (*slot, *inner, !matches!(cause, ViewCause::Reshaped))
        });
        let mut reported: HashSet<(usize, usize)> = HashSet::default();
        for ((slot, inner), cause) in places {
            let k = usize::from(slot);
            let Some(place) = args
                .get(k)
                .and_then(|a| call_arg_place(a, data))
                .and_then(|base| compose_param_place(base, inner))
            else {
                continue;
            };
            for (j, arg) in args.iter().enumerate() {
                if j == k || reported.contains(&(k, j)) {
                    continue;
                }
                // Only a parameter that can NAME an element is a hazard; a plain scalar or a
                // text copies, so there is nothing pinned to a position.  A scalar `&`
                // parameter links to the element itself (@FR-B-Ref-Lvalue), so it names one.
                let Some(attr) = cdef.attributes.get(j) else {
                    continue;
                };
                // A `&text` parameter handed a text field or element links that place too
                // (@PLN167 C3): the argument is then the place op itself, never the
                // `OpCreateStack` of a text variable, which names no element.
                let text_place_link = matches!(attr.typedef.base(),
                        Type::RefVar(inner) if matches!(inner.base(), Type::Text(_)))
                    && data.is_store_text_arg(arg);
                let scalar_link = text_place_link
                    || matches!(&attr.typedef, Type::RefVar(inner) if crate::data::is_scalar(inner));
                let ptp = match &attr.typedef {
                    Type::RefVar(inner) => inner.as_ref(),
                    other => other,
                };
                // `@FR-N-Shape` — `heap_def_nr`, which peels `τ?`: a nullable record or
                // struct-enum parameter aliases its element exactly as the dense one does.
                if ptp.heap_def_nr().is_none() && !scalar_link {
                    continue;
                }
                if !arg_references_element_in(arg, place, &def.code, function, data) {
                    continue;
                }
                reported.insert((k, j));
                let (does, why, after) = match cause {
                    ViewCause::Grown => (
                        "grows",
                        "and a container that outgrows its allocation moves every element",
                        "growth",
                    ),
                    _ => (
                        "removes from",
                        "which renumbers the remaining elements",
                        "removal",
                    ),
                };
                out.push(ReshapeRefusal {
                    file: file.to_string(),
                    line,
                    message: format!(
                        "cannot pass both `{cname}` and a reference into it to `{fname}` — \
                         `{fname}` {does} `{cparam}`, {why} while `{vparam}` still references \
                         one, so a write through `{vparam}` would be lost. Pass the INDEX \
                         instead and read the element again after the {after}",
                        cname = function.name(place.0),
                        fname = cdef.original_name(),
                        cparam = cdef.attributes[k].name,
                        vparam = attr.name,
                    ),
                });
            }
        }
    });
    out.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.message.cmp(&b.message)));
    out.dedup_by(|a, b| a.line == b.line && a.message == b.message);
    out
}
