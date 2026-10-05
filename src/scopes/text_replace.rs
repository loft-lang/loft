// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-H-TextReplace` — which text-field ASSIGNMENTS release the text they replace.
//!
//! The parser spells every assignment to a text field `OpSetTextReplace`, which writes over or
//! releases the block the slot holds (`Store::refill_str`).  That is sound only where nothing
//! still BORROWS the old block: a `text` parameter is handed the field's bytes, not a copy, so
//! `g(r.a, r)` whose body writes `q.a` would read its own parameter changed or released.  This
//! pass keeps the release where the absence of such a borrow is visible in the function itself,
//! and turns every other site back into `OpSetText`, the write that releases nothing — a site
//! declined here keeps the old block until its store dies, which is a leak and never a wrong
//! value.

use crate::data::{Data, Type, Value};
use crate::variables::Function;

/// Run over every definition, after the scope pass has settled each variable's deps.
pub(super) fn admit(data: &mut Data) {
    let replace = data.def_nr("OpSetTextReplace");
    let set = data.def_nr("OpSetText");
    if replace == u32::MAX || set == u32::MAX {
        return;
    }
    for d_nr in 0..data.definitions.len() {
        if !data.definitions[d_nr]
            .code
            .any_node(&mut |n| matches!(n, Value::Call(d, _) if *d == replace))
        {
            continue;
        }
        let mut code = std::mem::replace(&mut data.definitions[d_nr].code, Value::Null);
        visit(
            &mut code,
            false,
            replace,
            set,
            data,
            &data.definitions[d_nr].variables,
        );
        data.definitions[d_nr].code = code;
    }
}

/// `under_call` is whether an enclosing CALL is still evaluating its arguments: a `text`
/// argument already evaluated there is a borrow on the evaluation stack that no variable
/// records, so a write nested inside one (`f(r.a, { r.a = "x"; 1 })`) is declined.  A
/// statement in a block, a loop or a branch is not inside any argument list.
fn visit(n: &mut Value, under_call: bool, replace: u32, set: u32, data: &Data, vars: &Function) {
    // A `Span` only carries a source position: its child is in the same evaluation context.
    if let Value::Span(b) = n {
        return visit(&mut b.1, under_call, replace, set, data, vars);
    }
    let inner = match n {
        Value::Call(d, args) => {
            if *d == replace && (under_call || !admitted(args, data, vars)) {
                *d = set;
            }
            true
        }
        Value::CallRef(..) | Value::Parallel(_) | Value::Iter(..) => true,
        _ => under_call,
    };
    n.for_each_child_mut(&mut |c| visit(c, inner, replace, set, data, vars));
}

/// Is the write `OpSetTextReplace(place, fld, val)` free of a borrow of the block it replaces?
///
/// Yes when the place's record resolves, through the deps of the variables that reach it, to an
/// OWNER that is a local of this function (not a parameter, so no caller frame can hold a borrow
/// of its texts), and no `text` variable of this function depends on that owner (a borrow this
/// frame took — `x = r.a`, a `for c in r.a` walk, a `text?` local).  A place with no recognisable root, a chain
/// that branches or cycles, or an owner reached by a text borrow answers no: each is a shape
/// whose borrows this pass cannot see, and declining one costs only the release.
fn admitted(args: &[Value], data: &Data, vars: &Function) -> bool {
    let Some(root) = args
        .first()
        .and_then(|p| crate::use_analysis::place_root(p, data))
    else {
        return false;
    };
    let Some(owner) = owner_of(root, vars) else {
        return false;
    };
    if vars.is_argument(owner) {
        return false;
    }
    // `.base()`: a `text?` variable borrows exactly as a `text` one does.
    !(0..vars.count())
        .any(|v| matches!(vars.tp(v).base(), Type::Text(_)) && reaches(v, owner, vars))
}

/// Follow single deps from `v` to the variable that owns its store; `None` for a chain that
/// branches (two deps) or does not end within the table's size (a cycle).
fn owner_of(v: u16, vars: &Function) -> Option<u16> {
    let mut at = v;
    for _ in 0..=vars.count() {
        match vars.tp(at).depend().as_slice() {
            [] => return Some(at),
            [one] => at = *one,
            _ => return None,
        }
    }
    None
}

/// Does `v` depend on `owner`, directly or through other variables' deps?
fn reaches(v: u16, owner: u16, vars: &Function) -> bool {
    let mut seen = vec![false; usize::from(vars.count())];
    let mut todo = vars.tp(v).depend();
    while let Some(d) = todo.pop() {
        if d == owner {
            return true;
        }
        if let Some(s) = seen.get_mut(usize::from(d))
            && !*s
        {
            *s = true;
            todo.extend(vars.tp(d).depend());
        }
    }
    false
}
