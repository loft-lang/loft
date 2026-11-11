// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! `(N-Index)` — the locals that hold a vector's LENGTH for the rest of their function.
//!
//! `n = len(v); … v[n - 1]` reads the same element as `v[len(v) - 1]`, which `(N-Index)` trusts,
//! exactly when `n` still equals `len(v)` at the read.  That holds, on every path, when:
//!
//! - `n` is assigned once in the function, by that `len(v)`, and never handed to a `&` parameter;
//! - no statement that can run after it writes `v` — the variable, or for a field `s.v` the
//!   record `s` — nor any variable whose type views it, and no closure captures either.  What
//!   can run after it is the rest of each enclosing statement list and, for a loop around it, the
//!   whole loop: its next turn runs what stands before the assignment too.
//!
//! The answer is read off the function's PASS-1 body, which is whole when pass 2 parses the read
//! (a read can come before a write in the source and after it at run time, inside a loop).

use crate::data::{Data, Value};
use crate::parser::operators::{VecKey, vec_key};
use crate::variables::Function;

/// The `(local, variable)` pairs of `code` whose local holds the length of a vector reached
/// through that variable after its one assignment.  Pass 1 has not laid out a FIELD yet, so the
/// vector itself is named when pass 2 parses the assignment ([`settle`]).
pub(crate) fn length_locals(code: &Value, data: &Data, vars: &Function) -> Vec<(u16, u16)> {
    let mut sets: Vec<(u16, VecKey)> = Vec::new();
    code.walk(&mut |n| {
        if let Value::Set(v, rhs) = n
            && let Value::Call(op, args) = rhs.unspan()
            && args.len() == 1
            && is_len_call(data, *op)
            && let Some(key) = vec_key(&args[0], data)
        {
            sets.push((*v, key));
        }
    });
    sets.retain(|(n, key)| {
        assigned_once(code, *n)
            && !vars.is_argument(*n)
            && !vars.is_captured(*n)
            && !passed_by_reference(code, data, *n)
            && stable_after(code, data, vars, *n, base_var(key))
    });
    sets.into_iter()
        .map(|(n, key)| (n, base_var(&key)))
        .collect()
}

/// The vector a pass-2 assignment `n = len(<vector>)` names, when `n` is one of `found`'s locals
/// and the vector is reached through the variable recorded for it.
pub(crate) fn settle(code: &Value, data: &Data, found: &[(u16, u16)]) -> Option<(u16, VecKey)> {
    let Value::Set(n, rhs) = code.unspan() else {
        return None;
    };
    let Value::Call(op, args) = rhs.unspan() else {
        return None;
    };
    if args.len() != 1 || !is_len_call(data, *op) {
        return None;
    }
    let key = vec_key(&args[0], data)?;
    found
        .iter()
        .any(|(m, base)| m == n && *base == base_var(&key))
        .then_some((*n, key))
}

/// `@FR-R-Base`'s and `@FR-R-InRange`'s length-local form: the `(local, vector)` pairs of
/// function `d` whose local holds the length of a vector of the frame's own (`own_vector`),
/// named as the variable itself — so a loop over `0..n` walks `0..len(v)`.  One answer for the
/// IR pass and the C writer.  `LOFT_NO_LENGTH_LOCAL=1` answers none.
pub(crate) fn own_length_locals(data: &Data, d: u32, vars: &Function) -> Vec<(u16, u16)> {
    if crate::env_once!(std::env::var("LOFT_NO_LENGTH_LOCAL").is_ok_and(|v| v != "0")) {
        return Vec::new();
    }
    let def = data.def(d);
    let code = def.code();
    let mut found: Vec<(u16, u16)> = length_locals(code, data, vars)
        .into_iter()
        .filter(|(n, v)| {
            own_vector(def.attributes(), vars, *v) && holds_var_length(code, data, *n, *v)
        })
        .collect();
    if crate::env_once!(std::env::var("LOFT_NO_LOCKSTEP").is_ok_and(|v| v != "0")) {
        return found;
    }
    // A length local of `v` is one of each vector `v` moves in lockstep with, for as long as
    // nothing after it writes that vector either.
    let mut twins = Vec::new();
    for &(n, v) in &found {
        for w in lockstep_twins(code, data, vars, v) {
            if own_vector(def.attributes(), vars, w)
                && stable_after(code, data, vars, n, w)
                && !found.contains(&(n, w))
                && !twins.contains(&(n, w))
            {
                twins.push((n, w));
            }
        }
    }
    found.extend(twins);
    found
}

/// The vectors `v` moves in LOCKSTEP with: `len(w) == len(v)` at every statement boundary of
/// `code` outside an append pair (doc/claude/plans/211-rule-phases/d2-upper-bound.md § Lockstep
/// twins).  That holds when both are assigned once and not captured, their empty starts are
/// adjacent statements of one list, and every other write of either is one append to one
/// followed, as the very next statement, by one append to the other.  A statement such as
/// `n = len(v)` never runs between two adjacent statements, so `n` holds `len(w)` too.
pub(crate) fn lockstep_twins(code: &Value, data: &Data, vars: &Function, v: u16) -> Vec<u16> {
    let mut out = Vec::new();
    if !assigned_once(code, v) || vars.is_captured(v) {
        return out;
    }
    let mut starts = Vec::new();
    code.walk(&mut |node| {
        if let Value::Block(bl) | Value::Loop(bl) = node {
            start_pairs(&bl.operators, data, vars, &mut starts);
        } else if let Value::Insert(ops) = node {
            start_pairs(ops, data, vars, &mut starts);
        }
    });
    for (x, y, skip) in starts {
        let w = match (x == v, y == v) {
            (true, false) => y,
            (false, true) => x,
            _ => continue,
        };
        if out.contains(&w) || !assigned_once(code, w) || vars.is_captured(w) {
            continue;
        }
        let mut cache = crate::fxhash::FxHashMap::default();
        let lock = Lockstep {
            data,
            vars,
            v,
            w,
            skip: &skip,
        };
        if lock.only_paired_writes(code, &mut cache) {
            out.push(w);
        }
    }
    out
}

/// The empty start of a vector at `pl[j]`, with the companions it names: a work buffer's
/// `if OpRefIsNull(x) { mint } else { clear }`, or a local's `x = OpGetField(db, off, …)` whose
/// slot a following statement naming only `db` empties (`OpSetInt4(db, off, 0)`).
fn start_at(pl: &[&Value], j: usize, data: &Data, vars: &Function) -> Option<(u16, Vec<u16>)> {
    if let Some((x, others)) = crate::empty_slot::buffer_init(data, pl[j]) {
        return Some((x, others));
    }
    let Value::Set(x, rhs) = pl[j].unspan() else {
        return None;
    };
    if !matches!(vars.tp(*x).base(), crate::data::Type::Vector(..)) {
        return None;
    }
    let Value::Call(get, args) = rhs.unspan() else {
        return None;
    };
    if data.def(*get).name() != "OpGetField" {
        return None;
    }
    let (Some(Value::Var(db)), Some(Value::Int(off))) = (
        args.first().map(Value::unspan),
        args.get(1).map(Value::unspan),
    ) else {
        return None;
    };
    let comps = vec![*db];
    pl[j + 1..]
        .iter()
        .take_while(|op| names_only(op, &comps))
        .any(|op| crate::empty_slot::empties(data, op, (*db, *off)))
        .then_some((*x, comps))
}

/// Does `op` name no variable outside `comps`?
fn names_only(op: &Value, comps: &[u16]) -> bool {
    let mut named = crate::fxhash::FxHashSet::default();
    crate::parser::closures::collect_vars_in(op, &mut named);
    named.iter().all(|x| comps.contains(x))
}

/// The statement list without its line marks.
fn plain_ops(ops: &[Value]) -> Vec<&Value> {
    ops.iter()
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect()
}

/// Every two empty starts of one statement list with nothing between them but their
/// companions' statements, as `(x, y, statements of the two starts)`.
fn start_pairs(ops: &[Value], data: &Data, vars: &Function, out: &mut Vec<(u16, u16, Vec<usize>)>) {
    let pl = plain_ops(ops);
    let found: Vec<(usize, u16, Vec<u16>)> = (0..pl.len())
        .filter_map(|j| start_at(&pl, j, data, vars).map(|(x, c)| (j, x, c)))
        .collect();
    for pair in found.windows(2) {
        let [(j1, x, c1), (j2, y, c2)] = pair else {
            continue;
        };
        if x == y {
            continue;
        }
        let comps: Vec<u16> = c1.iter().chain(c2).copied().collect();
        if !pl[j1 + 1..*j2].iter().all(|op| names_only(op, &comps)) {
            continue;
        }
        let first = (0..*j1)
            .rev()
            .take_while(|k| names_only(pl[*k], &comps))
            .last()
            .unwrap_or(*j1);
        let last = (*j2 + 1..pl.len())
            .take_while(|k| names_only(pl[*k], &comps))
            .last()
            .unwrap_or(*j2);
        let skip = pl[first..=last]
            .iter()
            .map(|op| std::ptr::from_ref(*op) as usize)
            .collect();
        out.push((*x, *y, skip));
    }
}

/// [`lockstep_twins`]' write check for one candidate pair `v`, `w`.
struct Lockstep<'a> {
    data: &'a Data,
    vars: &'a Function,
    v: u16,
    w: u16,
    /// The statements of the two starts.
    skip: &'a [usize],
}

impl Lockstep<'_> {
    /// Is every write of `v` or `w` in `node` part of a start or of an append pair?
    fn only_paired_writes(
        &self,
        node: &Value,
        cache: &mut crate::fxhash::FxHashMap<u32, Vec<bool>>,
    ) -> bool {
        if self.skip.contains(&(std::ptr::from_ref(node) as usize)) {
            return true;
        }
        match node.unspan() {
            Value::Block(bl) | Value::Loop(bl) => self.list_ok(&bl.operators, cache),
            Value::Insert(ops) => self.list_ok(ops, cache),
            other => {
                if self.writes(other, cache) {
                    return false;
                }
                let mut ok = true;
                other.for_each_child(&mut |c| {
                    ok = ok && self.only_paired_writes(c, cache);
                });
                ok
            }
        }
    }

    fn writes(&self, node: &Value, cache: &mut crate::fxhash::FxHashMap<u32, Vec<bool>>) -> bool {
        writes_node(self.data, self.vars, node, self.v, cache)
            || writes_node(self.data, self.vars, node, self.w, cache)
    }

    fn list_ok(&self, ops: &[Value], cache: &mut crate::fxhash::FxHashMap<u32, Vec<bool>>) -> bool {
        let pl = plain_ops(ops);
        let mut j = 0;
        while j < pl.len() {
            if let Some(len) = self.pair_at(&pl[j..], cache) {
                j += len;
                continue;
            }
            if !self.only_paired_writes(pl[j], cache) {
                return false;
            }
            j += 1;
        }
        true
    }

    /// The statements an append pair at the head of `pl` spans: an append to one of `v`, `w`
    /// and then one to the other, neither operand writing either, and the second unable to
    /// leave (no `break`, `continue`, `return` or `yield`) between the two pushes.
    fn pair_at(
        &self,
        pl: &[&Value],
        cache: &mut crate::fxhash::FxHashMap<u32, Vec<bool>>,
    ) -> Option<usize> {
        let (x1, e1, n1) = self.append_at(pl)?;
        let (x2, e2, n2) = self.append_at(&pl[n1..])?;
        if x1 == x2 || ![x1, x2].contains(&self.v) || ![x1, x2].contains(&self.w) {
            return None;
        }
        let clean = |e: &[Value], cache: &mut crate::fxhash::FxHashMap<u32, Vec<bool>>| {
            e.iter()
                .all(|a| !a.any_node(&mut |n| self.writes(n, cache)))
        };
        let leaves = e2.iter().any(|a| {
            a.any_node(&mut |n| {
                matches!(
                    n,
                    Value::Break(_) | Value::Continue(_) | Value::Return(_) | Value::Yield(_)
                )
            })
        });
        (clean(e1, cache) && clean(e2, cache) && !leaves).then_some(n1 + n2)
    }

    /// One append at the head of `pl`: an optional `OpPreAllocVector(x, 1, _)`, then
    /// `OpPush*(x, …)` → `(x, the push's other operands, statements)`.
    fn append_at<'b>(&self, pl: &[&'b Value]) -> Option<(u16, &'b [Value], usize)> {
        let name = |op: u32| self.data.def(op).name();
        let mut k = 0;
        let mut pre = None;
        if let Value::Call(op, args) = pl.first()?.unspan()
            && name(*op) == "OpPreAllocVector"
            && let [Value::Var(x), cap, _] = args.as_slice()
            && matches!(cap.unspan(), Value::Int(1))
        {
            pre = Some(*x);
            k = 1;
        }
        let Value::Call(op, args) = pl.get(k)?.unspan() else {
            return None;
        };
        if !name(*op).starts_with("OpPush") {
            return None;
        }
        let Value::Var(x) = args.first()?.unspan() else {
            return None;
        };
        if pre.is_some_and(|p| p != *x) {
            return None;
        }
        Some((*x, &args[1..], k + 1))
    }
}

/// Is `v` a vector of the frame's own — a local, or a `@FR-R-WorkBuffer` parameter (a local
/// promoted to a scratch store the caller supplies) — viewing, through its deps, no other
/// parameter?  `length_locals` answers for the writes through `v` and through what it views;
/// a declared parameter can name the same vector as another parameter or as a field of one
/// (`f(a, a)`, `f(s.items, s)`), and a write through that other name shortens `v` unseen.
/// A parameter's attribute is the one at its place among the parameters, which is its
/// variable number only when no local is numbered between two parameters.
pub(crate) fn own_vector(attrs: &[crate::data::Attribute], vars: &Function, v: u16) -> bool {
    let params = vars.arguments();
    let user_param = |x: u16| {
        vars.is_argument(x)
            && !params
                .iter()
                .position(|p| *p == x)
                .and_then(|at| attrs.get(at))
                .is_some_and(|a| a.work_buffer)
    };
    let mut seen = vec![v];
    let mut i = 0;
    while i < seen.len() {
        let x = seen[i];
        if user_param(x) {
            return false;
        }
        for d in vars.tp(x).depend() {
            if !seen.contains(&d) {
                seen.push(d);
            }
        }
        i += 1;
    }
    true
}

/// Is `n`'s one assignment in `code` the length of the variable `v` ITSELF (`n = len(v)`), not
/// of a field reached through it?  `length_locals` keys a field's length by its record, and a
/// reader holding the vector needs the vector.
pub(crate) fn holds_var_length(code: &Value, data: &Data, n: u16, v: u16) -> bool {
    code.any_node(&mut |node| {
        matches!(node, Value::Set(m, rhs) if *m == n
            && matches!(rhs.unspan(), Value::Call(op, args) if args.len() == 1
                && is_len_call(data, *op)
                && matches!(vec_key(&args[0], data), Some(VecKey::Var(x)) if x == v)))
    })
}

/// Is `op` a `len` on a collection — the free `len`, a `t_<N><Type>_len` method, or the
/// `OpLengthVector` the method lowers to (the spelling a writer meets in the finished IR)?
pub(crate) fn is_len_call(data: &Data, op: u32) -> bool {
    let n = data.def(op).name.as_str();
    n == "n_len" || n == "OpLengthVector" || (n.starts_with("t_") && n.ends_with("_len"))
}

/// The variable a write to the keyed vector goes through.
fn base_var(key: &VecKey) -> u16 {
    match key {
        VecKey::Var(v) | VecKey::Field(v, _, _) | VecKey::OpenField(v, _, _) => *v,
    }
}

/// Is `n` an argument of a call whose parameter is a `&` — a write the call makes?
fn passed_by_reference(code: &Value, data: &Data, n: u16) -> bool {
    code.any_node(&mut |node| {
        let Value::Call(op, args) = node else {
            return false;
        };
        let attrs = data.def(*op).attributes();
        args.iter().enumerate().any(|(i, a)| {
            matches!(a.unspan(), Value::Var(x) if *x == n)
                && attrs
                    .get(i)
                    .is_some_and(|at| matches!(at.typedef.base(), crate::data::Type::RefVar(_)))
        })
    })
}

/// Does no statement that can run after the assignment of `n` write `base` — by a
/// reassignment, or a write through `base`, a variable viewing it, or one it views?  Binding a
/// new view of it (`e = v[i]`) writes nothing.
fn stable_after(code: &Value, data: &Data, vars: &Function, n: u16, base: u16) -> bool {
    if vars.is_captured(base) {
        return false;
    }
    let mut after: Vec<&Value> = Vec::new();
    if !statements_after(code, n, &mut after) {
        if std::env::var_os("LOFT_DBG_LL").is_some() {
            eprintln!("ll-not-a-statement n={n}");
        }
        return false;
    }
    let mut cache = crate::fxhash::FxHashMap::default();
    !after
        .iter()
        .any(|stmt| stmt.any_node(&mut |node| writes_node(data, vars, node, base, &mut cache)))
}

/// Does `node` ITSELF (not its operands) write `base`: a reassignment of it, a tuple put or a
/// call writing an argument that reaches it (the variable, one viewing it, one it views), or a
/// call through a reference, which may write anything?  The one write test of this file.
fn writes_node(
    data: &Data,
    vars: &Function,
    node: &Value,
    base: u16,
    cache: &mut crate::fxhash::FxHashMap<u32, Vec<bool>>,
) -> bool {
    let reach = |w: u16| {
        w == base || vars.tp(w).depend().contains(&base) || vars.tp(base).depend().contains(&w)
    };
    let touches = |arg: &Value| {
        let mut named = crate::fxhash::FxHashSet::default();
        crate::parser::closures::collect_vars_in(arg, &mut named);
        named.into_iter().any(reach)
    };
    match node.unspan() {
        Value::Set(w, _) => *w == base,
        Value::TuplePut(w, _, _) => reach(*w),
        Value::Call(op, args) => {
            let def = data.def(*op);
            let attrs = def.attributes();
            let callee = if *def.code() == Value::Null {
                Vec::new()
            } else {
                crate::parser::closures::callee_param_writes(*op, data, cache)
            };
            args.iter().enumerate().any(|(i, a)| {
                let writes = (i == 0 && crate::parser::op_writes_first_arg(def.name()))
                    || (i == 1 && def.name() == "OpCopyRecord")
                    || attrs.get(i).is_some_and(|at| {
                        matches!(at.typedef.base(), crate::data::Type::RefVar(_))
                    })
                    || callee.get(i).copied().unwrap_or(false);
                let hit = writes && touches(a);
                if hit && std::env::var_os("LOFT_DBG_LL").is_some() {
                    eprintln!("ll-write base={base} by {} arg {i}", def.name());
                }
                hit
            })
        }
        Value::CallRef(..) => true,
        _ => false,
    }
}

/// Collect into `after` every statement that can run after the assignment of `n` in `node`,
/// level by level up the statement lists that hold it — a loop's whole body.  `false` when the
/// assignment is not found as a statement.
fn statements_after<'a>(node: &'a Value, n: u16, after: &mut Vec<&'a Value>) -> bool {
    match node.unspan() {
        Value::Set(v, _) if *v == n => true,
        Value::Block(bl) => statements_after_in(&bl.operators, n, after),
        Value::Loop(bl) => {
            let found = statements_after_in(&bl.operators, n, after);
            if found {
                after.extend(&bl.operators);
            }
            found
        }
        Value::Insert(ops) => statements_after_in(ops, n, after),
        Value::If(_, then, els) => {
            statements_after(then, n, after) || statements_after(els, n, after)
        }
        _ => false,
    }
}

/// [`statements_after`] over one statement list.
fn statements_after_in<'a>(ops: &'a [Value], n: u16, after: &mut Vec<&'a Value>) -> bool {
    for (i, op) in ops.iter().enumerate() {
        if statements_after(op, n, after) {
            after.extend(&ops[i + 1..]);
            return true;
        }
    }
    false
}

/// Is `v` assigned exactly once in `code`?
pub(crate) fn assigned_once(code: &Value, v: u16) -> bool {
    let mut count = 0;
    code.walk(&mut |n| {
        if matches!(n, Value::Set(w, _) if *w == v) {
            count += 1;
        }
    });
    count == 1
}

/// Is `v` a vector element read — `OpGetVector(v, size, i)`, or a plain element's read at 0 of
/// one (`OpGetInt(OpGetVector(…), 0)`)?
pub(crate) fn is_element_read(v: &Value, data: &Data) -> bool {
    let Value::Call(op, args) = v.unspan() else {
        return false;
    };
    let name = data.def(*op).name.as_str();
    name == "OpGetVector"
        || (name.starts_with("OpGet")
            && matches!(args.first().map(Value::unspan),
                Some(Value::Call(inner, _)) if data.def(*inner).name == "OpGetVector"))
}
