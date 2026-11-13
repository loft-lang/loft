// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Rewrites that write a JOIN out per arm before the scan reads it: a join copied into a
//! container, a coalesce chain, and the reassignments a first scan wrote out per arm.

use super::Scopes;
use super::handoff::{appends_to_element, copy_hands_off, var_copy_owns};
use crate::data::{Block, Data, Type, Value, v_set};
use crate::variables::Function;

pub(super) fn branch_tail_vars(node: &Value) -> Vec<u16> {
    fn walk(n: &Value, out: &mut Vec<u16>) {
        match n.unspan() {
            Value::Var(x) => {
                if !out.contains(x) {
                    out.push(*x);
                }
            }
            Value::If(_, t, f) => {
                walk(t, out);
                walk(f, out);
            }
            // Written out as its call, which hands off no local.
            Value::Block(bl) if bl.name == crate::parser::Parser::JOIN_ARM_OWNER => {}
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last() {
                    walk(last, out);
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last() {
                    walk(last, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

/// `formal/heap-history.md` D-heap-15 — write a join COPIED into a container out per arm:
/// `OpCopyRecord(if c { a } else { b }, dest, tp)` becomes
/// `if c { OpCopyRecord(a, dest, tp) } else { OpCopyRecord(b, dest, tp) }`.
///
/// `Hold { h: a ?? b }` and `v += [a ?? b]` move whichever value the join chose into the
/// container (`(H-Move)`), and the container releases it.  Copied as one value, the join names no
/// variable, so nothing was handed over on any path and the chosen arm's source released it a
/// second time.  Written out, each arm is the plain copy the author's own
/// `if c { v += [a] } else { v += [b] }` makes, and [`copy_record_handoff`] and the per-path flag
/// (`arm_container_handoffs`) decide it as they decide that spelling.
///
/// Only a copy that hands a release over is written out — into a place a droppable-owning
/// container's cascade reaches, or an appended element ([`copy_hands_off`],
/// [`appends_to_element`]).  EVERY copy of a join, written out or not, gives an arm that is a
/// bare call minting a record the owner the parser gives it in a view-typed join
/// (`{ __ref_N = call; __ref_N }`): the store the call mints lives only in the value it answers,
/// since the buffer it is handed starts as the null sentinel, and the copy only reads it —
/// `v += [a ?? mk(n)]` leaked one store per evaluation for every record type.
/// `@FR-H-Move` — a `??` CHAIN rebinding a record local that it names as one of its values,
/// rewritten right-associated before any analysis reads the function: `a = a ?? b ?? d` and
/// `a = x ?? a ?? d` become `if present(p) { p } else { q ?? d }`.
///
/// `??` is left-associative, so `(p ?? q) ?? d` hoists its subject `p ?? q` into a `__ncc_N`
/// temp, and the local's own value then reaches the binding through that temp rather than as
/// an arm.  The per-arm write-out, whose identity arm keeps a record the local already holds
/// (`sink_set_into_arms`), could not see it.  Neither could the owner witness the oracle gave
/// the local instead, and the two backends put the chain's one copy in different places, so it
/// lost the kept record's release on `--native` (D-heap-28).  In `If(present(p), p, q)` only
/// the `q` arm can be absent, so `(p ?? q) ?? d` equals `if present(p) { p } else { q ?? d }`,
/// with the same value, the same operands evaluated in the same order, and `d` still written
/// once.  EVERY chain is rewritten, not only one that names its destination (loft#1612): the
/// hoisted form binds the destination to the temp, which VIEWS the operand it chose, where
/// `(B-Copy)` gives the destination a copy — so `b = x ?? y ?? d` shared `x`'s store, on the
/// interpreter for a record and on both backends for a vector.  Right-associated, every operand
/// is an arm, and an arm's bind is that copy.  An operand that is not a variable keeps a temp of
/// its own, so it is still evaluated once, and the arm binds the temp: a temp holding a call's
/// own store is adopted rather than viewed.  That temp is the one the hoisted form gave it,
/// reused — minting another loses what the parse decided about it, which for an owned call
/// result is the release of its store.  A chain with an operand the parse left without a temp
/// and that this cannot give one keeps its form.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub(super) fn reassociate_coalesce_chains(code: &mut Value, function: &mut Function, data: &Data) {
    let present = data.def_nr("OpConvBoolFromRef");
    if present == u32::MAX {
        return;
    }
    // A chain's presence test, in both spellings: `OpConvBoolFromRef(v)` for a record, and
    // `OpNot(OpVectorIsNull(v))` for a collection (loft#1612 — read only the first, a vector
    // chain was never recognised as one).
    let not_op = data.def_nr("OpNot");
    let vec_null = data.def_nr("OpVectorIsNull");
    let is_present = move |c: &Value, v: u16| {
        let names = |x: &Value| matches!(x.unspan(), Value::Var(y) if *y == v);
        match c.unspan() {
            Value::Call(d, a) if *d == present => matches!(a.as_slice(), [x] if names(x)),
            Value::Call(d, a) if *d == not_op && not_op != u32::MAX => {
                matches!(a.as_slice(), [inner]
                    if matches!(inner.unspan(), Value::Call(n, b)
                        if *n == vec_null && vec_null != u32::MAX
                            && matches!(b.as_slice(), [x] if names(x))))
            }
            _ => false,
        }
    };
    fn walk(n: &mut Value, f: &mut dyn FnMut(&Value) -> Option<Value>) {
        n.for_each_child_mut(&mut |c| walk(c, f));
        if let Some(new) = f(n) {
            *n = new;
        }
    }
    // The chain with its head operand removed: the innermost `if present(p) { p } else { q }`
    // replaced by `q`, every level above it kept as it is.
    fn strip_head(val: &Value) -> Option<Value> {
        match val.unspan() {
            Value::If(_, _, alt) => Some((**alt).clone()),
            Value::Block(bl) if bl.name == "ncc" => {
                let [first, tail] = bl.operators.as_slice() else {
                    return None;
                };
                let Value::Set(tmp, subject) = first.unspan() else {
                    return None;
                };
                Some(Value::Block(Box::new(Block {
                    name: "ncc",
                    operators: vec![v_set(*tmp, strip_head(subject)?), tail.clone()],
                    result: bl.result.clone(),
                    scope: bl.scope,
                    var_size: bl.var_size,
                })))
            }
            _ => None,
        }
    }
    // The operands of a hoisted `??` chain, in order, or `None` where `val` is not one.
    // `(p ?? q) ?? r` is `ncc { t = <p ?? q>; if present(t) { t } else { r } }`, and its subject
    // is either the base `if present(p) { p } else { q }` or a shorter chain of the same shape.
    fn operands(
        val: &Value,
        is_present: &dyn Fn(&Value, u16) -> bool,
        out: &mut Vec<(Value, u16)>,
    ) -> bool {
        match val.unspan() {
            Value::If(c, head, alt) if matches!(head.unspan(), Value::Var(p) if is_present(c, *p)) =>
            {
                out.push(((**head).clone(), u16::MAX));
                // …and the alternative is itself a chain where four or more operands were
                // written left-associated (loft#1612): read through it, so every operand
                // becomes an arm rather than one nested `if` that is not a variable.
                if !operands(alt, is_present, out) {
                    out.push(((**alt).clone(), u16::MAX));
                }
                true
            }
            Value::Block(bl) if bl.name == "ncc" => {
                let [first, tail] = bl.operators.as_slice() else {
                    return false;
                };
                let (Value::Set(tmp, subject), Value::If(c, held, rest)) =
                    (first.unspan(), tail.unspan())
                else {
                    return false;
                };
                if !is_present(c, *tmp) || !matches!(held.unspan(), Value::Var(x) if x == tmp) {
                    return false;
                }
                // A subject that is not itself a chain IS the chain's first operand, and the
                // temp beside it is what holds it (loft#1612: `f() ?? x ?? d`, whose first
                // operand is a call, was not read as a chain at all).  The temp is carried so
                // the rewrite REUSES it: minting another loses what the parse gave this one,
                // which for an owned call result is the release of its store.
                if !operands(subject, is_present, out) {
                    out.push(((**subject).clone(), *tmp));
                }
                out.push(((**rest).clone(), u16::MAX));
                true
            }
            _ => false,
        }
    }
    // The chain `ops` written right-associated: every operand but the last an arm of its own,
    // `if present(p) { p } else { … }`.  `None` where an operand other than the last is not a
    // variable — one of those needs a temp of its own, and such a chain keeps its form.
    fn right_assoc(
        ops: &[(Value, u16)],
        present: u32,
        tp: &Type,
        function: &mut Function,
        counter: &mut u32,
    ) -> Option<Value> {
        let (last, rest) = ops.split_last()?;
        let mut acc = last.0.clone();
        for (o, tmp) in rest.iter().rev() {
            // A VARIABLE is tested and bound where it stands: the arm's bind is the plain bind
            // that copies.
            if let Value::Var(x) = o.unspan() {
                acc = Value::If(
                    Box::new(Value::Call(present, vec![Value::Var(*x)])),
                    Box::new(o.clone()),
                    Box::new(acc),
                );
                continue;
            }
            // Anything else — a call, an element read — is evaluated ONCE inside its own arm,
            // through a temp, and the arm binds that temp: a temp holding a call's own store is
            // adopted rather than viewed.  The temp the hoisted form already gave this operand
            // is REUSED, because minting another loses what the parse gave that one (for an
            // owned call result, the release of its store); an operand the parse left without
            // one — a trivial arm's default — gets a fresh temp here.
            let held = if *tmp == u16::MAX {
                *counter += 1;
                let fresh = function.add_temp_var(&format!("__ncc_a{counter}"), tp);
                function.set_skip_free(fresh);
                fresh
            } else {
                *tmp
            };
            acc = Value::Block(Box::new(Block {
                name: "ncc",
                operators: vec![
                    v_set(held, o.clone()),
                    Value::If(
                        Box::new(Value::Call(present, vec![Value::Var(held)])),
                        Box::new(Value::Var(held)),
                        Box::new(acc),
                    ),
                ],
                result: tp.clone(),
                scope: 0,
                var_size: 0,
            }));
        }
        Some(acc)
    }
    let mut counter = 0_u32;
    // `LOFT_NO_COALESCE_REASSOC=1` keeps loft#1591's gate — only a chain that NAMES its
    // destination is right-associated — so a chain the AUTHOR parenthesised keeps the hoisted
    // form, whose temp views the operand it chose.  The first bisect step for a wrong value, a
    // leak or a double release out of a `??` chain written with parentheses.  The flat spelling
    // is right-associated by the PARSER now (loft#1612, `formal/grammar.md` (G-Assoc)) and does
    // not reach here at all; what is left for this pass is the grouping the author wrote.
    let only_self = crate::keys::no_coalesce_reassoc();
    let rewrite = |n: &Value, function: &mut Function, counter: &mut u32| -> Option<Value> {
        let Value::Set(dest, val) = n else {
            return None;
        };
        let dest = *dest;
        if !matches!(
            function.tp(dest).base(),
            Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
        ) {
            return None;
        }
        let dest_tp = function.tp(dest).clone();
        if !matches!(val.unspan(), Value::Block(bl) if bl.name == "ncc") {
            return None;
        }
        let mut ops = Vec::new();
        if !operands(val, &is_present, &mut ops) {
            return None;
        }
        // The destination as the chain's HEAD: `a = a ?? rest` keeps the value `a` holds when it
        // is present, so it is `if present(a) { } else { a = rest }`, and `rest` no longer names
        // the destination.
        // Each arm is a block of its own, as `sink_set_into_arms` writes them: the rebind may
        // take a snapshot of the record it displaces, registered at the scope it runs in.
        if matches!(ops.first().map(|(o, _)| o.unspan()), Some(Value::Var(x)) if *x == dest) {
            let arm = |op: Value| {
                Value::Block(Box::new(Block {
                    name: "sunk arm",
                    operators: vec![op],
                    result: Type::Void,
                    scope: 0,
                    var_size: 0,
                }))
            };
            // The REST of the chain is right-associated too where its operands allow it
            // (loft#1612): stripped alone it keeps the hoisted form, whose temp views the
            // operand it chose — `a = a ?? x ?? d` with `a` absent bound `a` to `x`'s store.
            let rest = right_assoc(&ops[1..], present, &dest_tp, function, counter)
                .map_or_else(|| strip_head(val), Some)?;
            return Some(Value::If(
                Box::new(Value::Call(present, vec![Value::Var(dest)])),
                Box::new(arm(Value::Insert(Vec::new()))),
                Box::new(arm(v_set(dest, rest))),
            ));
        }
        // Right-associated all the way down, so every arm is a variable or the last default: a
        // middle operand that is not a variable would need a temp of its own again, and a
        // chain that has one keeps its form.
        //
        // For EVERY chain, not only one that names its destination (loft#1612).  The hoisted
        // form binds the destination to the temp, which VIEWS the chosen operand's store — the
        // interpreter for a record, both backends for a vector — where `(B-Copy)` gives the
        // destination a copy.  Right-associated, each operand is an ARM, and an arm's bind is
        // the plain bind that copies: the two-operand chain has always lowered that way
        // (`if let Value::Var(_) = code` in the `??` parse), and this gives the longer chains
        // the same shape.
        if only_self {
            return None;
        }
        Some(v_set(
            dest,
            right_assoc(&ops, present, &dest_tp, function, counter)?,
        ))
    };
    walk(code, &mut |n| rewrite(n, function, &mut counter));
}

#[expect(clippy::too_many_lines, reason = "inherited")]
pub(super) fn write_out_joined_copies(code: &mut Value, function: &Function, data: &Data) {
    let copy_d = data.def_nr("OpCopyRecord");
    if copy_d == u32::MAX {
        return;
    }
    fn writable(tail: &Value) -> bool {
        match tail.unspan() {
            Value::Var(_) | Value::Call(_, _) => true,
            Value::If(_, t, f) => writable(t) && writable(f),
            Value::Block(bl)
                if bl.name == crate::parser::Parser::JOIN_ARM_OWNER || bl.name == "Object" =>
            {
                true
            }
            Value::Block(bl) if !matches!(bl.result.base(), Type::Void | Type::Null) => {
                bl.operators.last().is_some_and(writable)
            }
            Value::Insert(ops) => ops.last().is_some_and(writable),
            _ => false,
        }
    }
    fn arm_block(arm: &mut Value) {
        if matches!(arm.unspan(), Value::Block(_) | Value::If(_, _, _)) {
            return;
        }
        let inner = std::mem::replace(arm, Value::Null);
        *arm = Value::Block(Box::new(Block {
            name: "sunk arm",
            operators: vec![inner],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }));
    }
    struct JoinCopy<'a> {
        op: u32,
        dest: &'a Value,
        tp: &'a Value,
        function: &'a Function,
        data: &'a Data,
    }
    impl JoinCopy<'_> {
        fn wrap(&self, node: &mut Value) {
            let source = std::mem::replace(node, Value::Null);
            *node = Value::Call(self.op, vec![source, self.dest.clone(), self.tp.clone()]);
        }
        /// The owner block for a bare minting call, or `None` where the call is not one.
        fn owned(&self, node: &Value) -> Option<Value> {
            let Value::Call(fd, args) = node.unspan() else {
                return None;
            };
            let def = self.data.def(*fd);
            if !def.is_loft_defined()
                || !matches!(
                    def.returned().peel_optional().0,
                    Type::Reference(_, _) | Type::Enum(_, true, _)
                )
            {
                return None;
            }
            let buf = args.iter().find_map(|a| match a.unspan() {
                Value::Var(v) if self.function.is_caller_hidden_buf(*v) => Some(*v),
                _ => None,
            })?;
            Some(Value::Block(Box::new(Block {
                name: crate::parser::Parser::JOIN_ARM_OWNER,
                operators: vec![v_set(buf, node.clone()), Value::Var(buf)],
                result: self.function.tp(buf).clone(),
                scope: 0,
                var_size: 0,
            })))
        }
        /// Give every bare minting call arm of the join its owner block, and nothing else.
        fn own_calls(&self, node: &mut Value) {
            if let Some(owned) = self.owned(node) {
                *node = owned;
                return;
            }
            match node {
                Value::Span(b) => self.own_calls(&mut b.1),
                Value::If(_, t, f) => {
                    self.own_calls(t);
                    self.own_calls(f);
                }
                Value::Block(bl) if bl.name != crate::parser::Parser::JOIN_ARM_OWNER => {
                    if let Some(last) = bl.operators.last_mut() {
                        self.own_calls(last);
                    }
                }
                Value::Insert(ops) => {
                    if let Some(last) = ops.last_mut() {
                        self.own_calls(last);
                    }
                }
                _ => {}
            }
        }
        fn sink(&self, node: &mut Value) {
            if let Some(owned) = self.owned(node) {
                *node = owned;
                self.wrap(node);
                return;
            }
            match node {
                Value::Span(b) => self.sink(&mut b.1),
                Value::If(_, t, f) => {
                    self.sink(t);
                    self.sink(f);
                    arm_block(t);
                    arm_block(f);
                }
                Value::Block(bl)
                    if bl.name == crate::parser::Parser::JOIN_ARM_OWNER || bl.name == "Object" =>
                {
                    self.wrap(node);
                }
                Value::Block(bl) => {
                    if let Some(last) = bl.operators.last_mut() {
                        self.sink(last);
                    }
                    bl.result = Type::Void;
                }
                Value::Insert(ops) => {
                    if let Some(last) = ops.last_mut() {
                        self.sink(last);
                    }
                }
                _ => self.wrap(node),
            }
        }
    }
    fn visit(n: &mut Value, copy_d: u32, function: &Function, data: &Data) {
        n.for_each_child_mut(&mut |c| visit(c, copy_d, function, data));
        let node = n.unspan_mut();
        let Value::Call(d, args) = &*node else {
            return;
        };
        if *d != copy_d
            || args.len() != 3
            || !Scopes::is_value_branch(&args[0])
            || !writable(&args[0])
            || matches!(args[2].unspan(), Value::Int(tp) if tp & 0x8000 != 0)
        {
            return;
        }
        let hands_over = copy_hands_off(&args[1], function, data)
            || appends_to_element(&args[1], function, data);
        let Value::Call(_, args) = std::mem::replace(node, Value::Null) else {
            unreachable!();
        };
        let [mut branch, dest, tp] = <[Value; 3]>::try_from(args).unwrap_or_else(|_| {
            unreachable!("an OpCopyRecord carries three arguments");
        });
        let copy = JoinCopy {
            op: copy_d,
            dest: &dest,
            tp: &tp,
            function,
            data,
        };
        if hands_over {
            copy.sink(&mut branch);
            *node = branch;
        } else {
            // No release to hand over, but the store a bare call arm mints still needs an owner:
            // the copy reads it and nothing else does.
            copy.own_calls(&mut branch);
            *node = Value::Call(copy_d, vec![branch, dest, tp]);
        }
    }
    visit(code, copy_d, function, data);
}

/// Rewrites the reassignments a first scan wrote out per arm into that per-arm statement form, in
/// `code` itself, and removes from `vars` the deps the parser gave each such binding for its local
/// arms.  A rescan from the result reads, in every analysis that runs before the scan, what the
/// author's own per-arm spelling gives it (`formal/binding.md` D-bind-34, `heap-history.md` D-heap-7).
/// Returns whether anything was rewritten.
///
/// Each site is found by the address of its `Set`'s value node, which the scan recorded while
/// reading this same tree, together with the variable — the scan's own decision, never
/// re-derived: a structural "is this a reassignment" disagreed with the scan at 52 of 899 corpus
/// sites.  Children are rewritten before their parent, so a recorded site nested inside another's
/// arm still has its recorded address when it is reached.  A site the scan reached only inside a
/// tree it had built itself is not in `code`; the rescan writes that one out as the scan did.
pub(super) fn rewrite_written_out(
    code: &mut Value,
    vars: &mut Function,
    data: &Data,
    written: &[(usize, u16)],
) -> bool {
    fn walk(
        n: &mut Value,
        vars: &mut Function,
        data: &Data,
        written: &[(usize, u16)],
        hit: &mut bool,
    ) {
        n.for_each_child_mut(&mut |c| walk(c, vars, data, written, hit));
        let stmt = match &*n {
            Value::Set(t, val)
                if written.contains(&(std::ptr::from_ref::<Value>(val).addr(), *t)) =>
            {
                let stmt = Scopes::sink_set_into_arms(*t, *t, val, vars, data, true);
                if stmt.is_some() {
                    for src in branch_tail_vars(val) {
                        if var_copy_owns(vars, data, *t, src) {
                            vars.make_independent(*t, src);
                        }
                    }
                }
                stmt
            }
            _ => None,
        };
        if let Some(stmt) = stmt {
            *n = stmt;
            *hit = true;
        }
    }
    let mut hit = false;
    walk(code, vars, data, written, &mut hit);
    hit
}
