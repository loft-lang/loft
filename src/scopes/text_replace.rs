// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-H-TextReplace` — which text-field ASSIGNMENTS release the text they replace.
//!
//! The parser spells every assignment to a text field `OpSetTextReplace`, which writes over or
//! releases the block the slot holds (`Store::refill_str`).  That is sound only where nothing
//! still BORROWS the old block: a `text` parameter is handed the field's bytes, not a copy, so
//! `g(r.a, r)` whose body writes `q.a` would read its own parameter changed or released.  This
//! pass keeps the release where the absence of such a borrow is proven — in the writing frame,
//! and through a parameter in every frame that can call it ([`safe_params`]) — and turns every
//! other site back into `OpSetText`, the write that releases nothing — a site
//! declined here keeps the old block until its store dies, which is a leak and never a wrong
//! value.

use crate::data::{Data, Definition, Type, Value};
use crate::variables::Function;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

/// Run over the functions this scope pass checked (`fresh`), after it has settled each
/// variable's deps.  A function checked by an earlier pass — the standard library, on every
/// later compile — was decided then and is not walked again.
pub(super) fn admit(data: &mut Data, fresh: &[u32]) {
    let replace = data.def_nr("OpSetTextReplace");
    let set = data.def_nr("OpSetText");
    if replace == u32::MAX || set == u32::MAX {
        return;
    }
    let safe = safe_params(data, fresh, replace);
    for &d_nr in fresh {
        let d = d_nr as usize;
        if !data.definitions[d]
            .code
            .any_node(&mut |n| matches!(n, Value::Call(c, _) if *c == replace))
        {
            continue;
        }
        let mut code = std::mem::replace(&mut data.definitions[d].code, Value::Null);
        let facts = Facts::of(&data.definitions[d].variables);
        let at = Site {
            fn_nr: d_nr,
            safe: &safe,
            replace,
            set,
        };
        visit(&mut code, false, &at, data, &facts);
        data.definitions[d].code = code;
    }
}

/// What one function's variable table says about ownership, read once per function: each
/// variable's deps (to find the owner of a place) and whether it is a parameter.  A text
/// VARIABLE is no borrow here: on the interpreter a local `text` is an owned copy (`x = r.a`,
/// a `??` discharge, a `for c in r.a` walk temporary are each set from `OpGetText` and freed
/// with `OpFreeText`), and the native slices that do borrow a store's text (`@FR-R-TextBorrow`)
/// are declined over any block in which a write that is not in place can run — a text
/// release is not in place.  What borrows across a release is a `text` PARAMETER and an
/// argument already evaluated, and both are checked where the release is admitted.
struct Facts<'a> {
    deps: Vec<Cow<'a, [u16]>>,
    argument: Vec<bool>,
}

impl<'a> Facts<'a> {
    /// The deps are BORROWED from the table (`Type::deps_ref`); only a tuple, whose deps are
    /// the union of its members', is built (`Type::depend`).
    fn of(vars: &'a Function) -> Facts<'a> {
        let deps: Vec<Cow<'a, [u16]>> = (0..vars.count())
            .map(|v| match vars.tp(v).deps_ref() {
                Some(d) => Cow::Borrowed(d.as_slice()),
                None if matches!(vars.tp(v).base(), Type::Tuple(_)) => {
                    Cow::Owned(vars.tp(v).depend())
                }
                None => Cow::Borrowed(&[][..]),
            })
            .collect();
        let argument = (0..vars.count()).map(|v| vars.is_argument(v)).collect();
        Facts { deps, argument }
    }

    /// Follow single deps from `v` to the variable that owns its store; `None` for a chain
    /// that branches (two deps) or does not end within the table's size (a cycle).
    fn owner_of(&self, v: u16) -> Option<u16> {
        let mut at = v;
        for _ in 0..=self.deps.len() {
            match &**self.deps.get(usize::from(at))? {
                [] => return Some(at),
                [one] => at = *one,
                _ => return None,
            }
        }
        None
    }
}

/// `under_call` is whether an enclosing call has already evaluated an argument that may BORROW
/// a text — a field read, a slice, a call answering text, a `text` parameter — and holds it on
/// the evaluation stack while this node runs: a write nested there (`f(r.a, { r.a = "x"; 1 })`)
/// is declined.  An argument evaluated before is a borrow only when [`borrows_text`] says so: the
/// work text a format string appends into is a local, an owned copy, so `"{set(r)}"` is clean.
/// A statement in a block, a loop or a branch is not inside any argument list.
fn visit(n: &mut Value, under_call: bool, at: &Site<'_>, data: &Data, facts: &Facts<'_>) {
    // A `Span` only carries a source position: its child is in the same evaluation context.
    if let Value::Span(b) = n {
        return visit(&mut b.1, under_call, at, data, facts);
    }
    let vars = &data.definitions[at.fn_nr as usize].variables;
    match n {
        Value::Call(d, args) => {
            if *d == at.replace && (under_call || !admitted(args, at, data, facts)) {
                *d = at.set;
            }
            let mut held = under_call;
            for a in args.iter_mut() {
                visit(a, held, at, data, facts);
                held = held || borrows_text(a, data, vars);
            }
        }
        Value::CallRef(..) | Value::Parallel(_) | Value::Iter(..) => {
            n.for_each_child_mut(&mut |c| visit(c, true, at, data, facts));
        }
        _ => n.for_each_child_mut(&mut |c| visit(c, under_call, at, data, facts)),
    }
}

/// Can evaluating `v` leave a BORROWED text on the evaluation stack?  Yes when it calls
/// anything that answers `text` (a field read, a slice, a user function — whether its answer
/// is a copy is the callee's business, so it counts) or reads a `text` parameter, whose bytes
/// may be a caller's field.  A local text variable is an owned copy ([`Facts`]), and every
/// other value — a number, a record reference — is no text at all.
fn borrows_text(v: &Value, data: &Data, vars: &Function) -> bool {
    v.any_node(&mut |n| match n {
        Value::Call(d, _) => matches!(data.def(*d).returned().base(), Type::Text(_)),
        Value::Var(x) => vars.is_argument(*x) && matches!(vars.tp(*x).base(), Type::Text(_)),
        _ => false,
    })
}

/// Is the write `OpSetTextReplace(place, fld, val)` free of a borrow of the block it replaces?
///
/// Yes when the place's record resolves, through the deps of the variables that reach it, to an
/// OWNER that is either a local of this function or a parameter [`safe_params`] proved no frame
/// above can hold a borrow of.  A place with no recognisable root, or a chain that branches or
/// cycles, answers no: each is a shape whose borrows this pass cannot see, and declining one
/// costs only the release.
fn admitted(args: &[Value], at: &Site<'_>, data: &Data, facts: &Facts<'_>) -> bool {
    let Some(root) = args
        .first()
        .and_then(|p| crate::use_analysis::place_root(p, data))
    else {
        return false;
    };
    let Some(owner) = facts.owner_of(root) else {
        return false;
    };
    !facts
        .argument
        .get(usize::from(owner))
        .copied()
        .unwrap_or(true)
        || at.safe.contains(&(at.fn_nr, owner))
}

/// The function being decided, the parameters proven safe across frames, and the two ops.
struct Site<'a> {
    fn_nr: u32,
    safe: &'a HashSet<(u32, u16)>,
    replace: u32,
    set: u32,
}

/// One direct call of a checked function: who calls, with which arguments, and whether the
/// call sits where no argument of an enclosing call is already evaluated (`clean`).
struct Call<'a> {
    caller: u32,
    args: &'a [Value],
    clean: bool,
}

/// `@FR-H-TextReplace` across frames — the parameters `(f, v)` through whose record `f` may
/// release a text it replaces: no frame on the stack when `f` runs can still hold a borrow of
/// that text.  A text is borrowed across frames in two ways only (a text variable is a copy —
/// [`Facts`]): a `text` parameter bound to the field, and an argument an enclosing call has
/// already evaluated.  So `(f, v)` holds when every `text` parameter of `f` is only ever the
/// value written, every caller of `f` is a visible direct call (none through a fn-ref), and at
/// each call the argument for `v` is not nested inside another call's arguments and is owned,
/// in the caller, either by a local or by a parameter `(caller, u)` that itself holds.
///
/// A greatest fixpoint: every candidate starts true and is struck when one call refutes it, so
/// a recursive function holds when its own calls keep the property.  Only the functions this
/// pass checked are candidates — a standard-library function is decided before the program
/// that calls it is read, so its release stays declined.
fn safe_params(data: &Data, fresh: &[u32], replace: u32) -> HashSet<(u32, u16)> {
    let in_fresh: HashSet<u32> = fresh.iter().copied().collect();
    let mut escaped: HashSet<u32> = HashSet::new();
    let mut calls: HashMap<u32, Vec<Call<'_>>> = HashMap::new();
    for &g in fresh {
        let code = &data.definitions[g as usize].code;
        code.walk(&mut |n| {
            if let Value::FnRef(f, _, _) = n
                && let Ok(f) = u32::try_from(*f)
            {
                escaped.insert(f);
            }
        });
        collect_calls(code, false, g, &in_fresh, data, &mut calls);
    }
    let facts: HashMap<u32, Facts<'_>> = fresh
        .iter()
        .map(|&d| (d, Facts::of(&data.definitions[d as usize].variables)))
        .collect();
    let mut safe: HashSet<(u32, u16)> = HashSet::new();
    for &f in fresh {
        let def = &data.definitions[f as usize];
        if escaped.contains(&f) || !calls.contains_key(&f) || !value_only_text_params(def, replace)
        {
            continue;
        }
        let fa = &facts[&f];
        for v in 0..def.variables.count() {
            let i = usize::from(v);
            if fa.argument[i] && !matches!(def.variables.tp(v).base(), Type::Text(_)) {
                safe.insert((f, v));
            }
        }
    }
    loop {
        let struck: Vec<(u32, u16)> = safe
            .iter()
            .copied()
            .filter(|&(f, v)| !holds(data, f, v, &calls[&f], &facts, &safe))
            .collect();
        if struck.is_empty() {
            return safe;
        }
        for k in struck {
            safe.remove(&k);
        }
    }
}

/// Does every call of `f` keep parameter `v` free of a borrow above `f`?
fn holds(
    data: &Data,
    f: u32,
    v: u16,
    calls: &[Call<'_>],
    facts: &HashMap<u32, Facts<'_>>,
    safe: &HashSet<(u32, u16)>,
) -> bool {
    let def = &data.definitions[f as usize];
    let name = def.variables.name(v);
    let Some(idx) = def.attributes.iter().position(|a| a.name == name) else {
        return false;
    };
    calls.iter().all(|c| {
        let cf = &facts[&c.caller];
        c.clean
            && c.args.len() == def.attributes.len()
            && c.args
                .get(idx)
                .and_then(|a| crate::use_analysis::place_root(a, data))
                .and_then(|r| cf.owner_of(r))
                .is_some_and(|o| {
                    let ou = usize::from(o);
                    !cf.argument.get(ou).copied().unwrap_or(true) || safe.contains(&(c.caller, o))
                })
    })
}

/// Every direct call of a function in `fresh` inside `n`, with whether an enclosing call holds
/// an argument that may borrow a text while it runs — the same context [`visit`] tracks.
fn collect_calls<'a>(
    n: &'a Value,
    under_call: bool,
    caller: u32,
    fresh: &HashSet<u32>,
    data: &'a Data,
    out: &mut HashMap<u32, Vec<Call<'a>>>,
) {
    match n {
        Value::Span(b) => collect_calls(&b.1, under_call, caller, fresh, data, out),
        Value::Call(d, args) => {
            if fresh.contains(d) {
                out.entry(*d).or_default().push(Call {
                    caller,
                    args,
                    clean: !under_call,
                });
            }
            let vars = &data.definitions[caller as usize].variables;
            let mut held = under_call;
            for a in args {
                collect_calls(a, held, caller, fresh, data, out);
                held = held || borrows_text(a, data, vars);
            }
        }
        Value::CallRef(..) | Value::Parallel(_) | Value::Iter(..) => {
            n.for_each_child(&mut |c| collect_calls(c, true, caller, fresh, data, out));
        }
        _ => n.for_each_child(&mut |c| collect_calls(c, under_call, caller, fresh, data, out)),
    }
}

/// Is every `text` parameter of `def` mentioned only as the value of a text-field assignment?
/// Such a parameter is read once, into the write, before the block it may borrow is released
/// (`OpSetTextReplace` copies its value first on both backends); any other mention — a read
/// after the write, a rebind, a walk, a capture — keeps the borrow alive past the release.
fn value_only_text_params(def: &Definition, replace: u32) -> bool {
    let vars = &def.variables;
    (0..vars.count())
        .filter(|&v| vars.is_argument(v) && matches!(vars.tp(v).base(), Type::Text(_)))
        .all(|p| {
            let mut total = 0usize;
            let mut as_value = 0usize;
            def.code.walk(&mut |n| match n {
                Value::Var(x)
                | Value::Set(x, _)
                | Value::TupleGet(x, _)
                | Value::TuplePut(x, _, _)
                | Value::Iter(x, _, _, _)
                | Value::CallRef(x, _)
                    if *x == p =>
                {
                    total += 1;
                }
                Value::Call(d, args)
                    if *d == replace
                        && matches!(args.get(2).map(Value::unspan), Some(Value::Var(x)) if *x == p) =>
                {
                    as_value += 1;
                }
                _ => {}
            });
            total == as_value
        })
}
