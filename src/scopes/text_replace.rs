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
use std::borrow::Cow;

/// Run over the functions this scope pass checked (`fresh`), after it has settled each
/// variable's deps.  A function checked by an earlier pass — the standard library, on every
/// later compile — was decided then and is not walked again.
pub(super) fn admit(data: &mut Data, fresh: &[u32]) {
    let replace = data.def_nr("OpSetTextReplace");
    let set = data.def_nr("OpSetText");
    if replace == u32::MAX || set == u32::MAX {
        return;
    }
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
        visit(&mut code, false, replace, set, data, &facts);
        data.definitions[d].code = code;
    }
}

/// What one function's variable table says about borrows, read once per function: each
/// variable's deps, whether it is a parameter, and whether a `text` variable depends on it,
/// directly or through other variables' deps.  `.base()`: a `text?` variable borrows exactly
/// as a `text` one does.
struct Facts<'a> {
    deps: Vec<Cow<'a, [u16]>>,
    argument: Vec<bool>,
    text_borrowed: Vec<bool>,
}

impl<'a> Facts<'a> {
    /// The deps are BORROWED from the table (`Type::deps_ref`); only a tuple, whose deps are
    /// the union of its members', is built (`Type::depend`).
    fn of(vars: &'a Function) -> Facts<'a> {
        let n = usize::from(vars.count());
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
        let mut text_borrowed = vec![false; n];
        let mut todo = Vec::new();
        for v in 0..vars.count() {
            if matches!(vars.tp(v).base(), Type::Text(_)) {
                todo.extend_from_slice(&deps[usize::from(v)]);
            }
        }
        while let Some(d) = todo.pop() {
            if let Some(b) = text_borrowed.get_mut(usize::from(d))
                && !*b
            {
                *b = true;
                todo.extend_from_slice(&deps[usize::from(d)]);
            }
        }
        Facts {
            deps,
            argument,
            text_borrowed,
        }
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

/// `under_call` is whether an enclosing CALL is still evaluating its arguments: a `text`
/// argument already evaluated there is a borrow on the evaluation stack that no variable
/// records, so a write nested inside one (`f(r.a, { r.a = "x"; 1 })`) is declined.  A
/// statement in a block, a loop or a branch is not inside any argument list.
fn visit(n: &mut Value, under_call: bool, replace: u32, set: u32, data: &Data, facts: &Facts<'_>) {
    // A `Span` only carries a source position: its child is in the same evaluation context.
    if let Value::Span(b) = n {
        return visit(&mut b.1, under_call, replace, set, data, facts);
    }
    let inner = match n {
        Value::Call(d, args) => {
            if *d == replace && (under_call || !admitted(args, data, facts)) {
                *d = set;
            }
            true
        }
        Value::CallRef(..) | Value::Parallel(_) | Value::Iter(..) => true,
        _ => under_call,
    };
    n.for_each_child_mut(&mut |c| visit(c, inner, replace, set, data, facts));
}

/// Is the write `OpSetTextReplace(place, fld, val)` free of a borrow of the block it replaces?
///
/// Yes when the place's record resolves, through the deps of the variables that reach it, to an
/// OWNER that is a local of this function (not a parameter, so no caller frame can hold a borrow
/// of its texts), and no `text` variable of this function depends on that owner (a borrow this
/// frame took — `x = r.a`, a `for c in r.a` walk, a `text?` local).  A place with no
/// recognisable root, a chain that branches or cycles, or an owner reached by a text borrow
/// answers no: each is a shape whose borrows this pass cannot see, and declining one costs only
/// the release.
fn admitted(args: &[Value], data: &Data, facts: &Facts<'_>) -> bool {
    let Some(root) = args
        .first()
        .and_then(|p| crate::use_analysis::place_root(p, data))
    else {
        return false;
    };
    let Some(owner) = facts.owner_of(root) else {
        return false;
    };
    let o = usize::from(owner);
    !facts.argument.get(o).copied().unwrap_or(true)
        && !facts.text_borrowed.get(o).copied().unwrap_or(true)
}
