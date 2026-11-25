// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! The scope check of the store records (`@PLN228`, STORE_RECORDS.md § The checks): every use
//! of a variable lies inside the variable's scope (`@FR-B-Scope`).
//!
//! A local's value ends with its scope: the scope's end releases it (`@FR-H-Drop`).  A pass that
//! homes a local in a narrower block than the one that reads it hands the reader a released
//! value, and the program answers wrong with nothing to report it at run time: in loft#2636 the
//! first arm's vector read back empty.  So this check reads the closed program's IR and reports
//! each use outside the variable's scope as a `store-out-of-scope` finding, beside the store
//! records' own kinds.
//!
//! Two forms outside the scope read no value, and are not uses: an initialisation to a constant
//! (a lift's entry `v = null`, which declares the variable for the frame) and a release (a block
//! whose value is a local is followed by that local's free, after the value's consumer).  The
//! live range a slot is placed by runs from the first write to the last mention whatever the
//! scopes say, so neither form costs a slot.
//!
//! **Independence.**  The check reads only the IR's block nesting (`Block::scope`) and the
//! variable table's `scope`, never a predicate of `src/scopes/`, the pass whose result it
//! checks.  It runs under `LOFT_OWN_ORACLE=check` and `=check-records`, the two modes
//! `scripts/own_sweep.py` scores; it changes nothing it reads.

use crate::data::{Data, DefType, Value};
use crate::variables::Function;

/// One use of a variable outside its scope.
struct Finding {
    var: u16,
    var_scope: u16,
    use_scope: u16,
}

/// The observer entry point, at the end of `close_program`.
pub fn observe(data: &Data) {
    let releases: Vec<u32> = (0..data.definitions())
        .filter(|&d| data.def(d).name().starts_with("OpFree"))
        .collect();
    let Ok(mode) = std::env::var("LOFT_OWN_ORACLE") else {
        return;
    };
    if mode != "check" && mode != "check-records" {
        return;
    }
    let mut reds = 0usize;
    for d in 0..data.definitions() {
        let def = data.def(d);
        if !matches!(def.def_type, DefType::Function) || matches!(def.code.unspan(), Value::Null) {
            continue;
        }
        for f in check_function(&def.variables, &def.code, &releases, data) {
            eprintln!(
                "RED {}: store-out-of-scope {} (v{}) of scope {} used in scope {} (@FR-B-Scope)",
                def.name(),
                def.variables.name(f.var),
                f.var,
                f.var_scope,
                f.use_scope
            );
            reds += 1;
        }
    }
    eprintln!("SCOPE-CHECK: {reds} RED finding(s)");
}

/// Every variable of `vars` that `code` uses outside the variable's scope, first use each.
fn check_function(vars: &Function, code: &Value, releases: &[u32], data: &Data) -> Vec<Finding> {
    // Scope 0 is the function's own: the body block opens scope 1 inside it.
    let mut walk = Walk {
        vars,
        releases,
        data,
        open: vec![0],
        found: Vec::new(),
    };
    walk.node(code);
    walk.found
}

struct Walk<'a> {
    vars: &'a Function,
    /// The `OpFree*` operators: a release of a variable reads no value of it.
    releases: &'a [u32],
    data: &'a Data,
    /// The scopes enclosing the node being read, outermost first: the function's own (0), then
    /// one per enclosing block.
    open: Vec<u16>,
    found: Vec<Finding>,
}

impl Walk<'_> {
    /// A use of variable `v` here.  An argument lives for the whole call; a variable whose
    /// scope was never set (`u16::MAX`) was never placed, so it has no slot to lose.
    fn use_of(&mut self, v: u16) {
        if self.vars.is_argument(v) {
            return;
        }
        let s = self.vars.scope(v);
        if s == u16::MAX || self.open.contains(&s) || self.found.iter().any(|f| f.var == v) {
            return;
        }
        self.found.push(Finding {
            var: v,
            var_scope: s,
            use_scope: self.open.last().copied().unwrap_or(u16::MAX),
        });
    }

    /// A constant written to a variable — a literal, a typed null (`OpConv*FromNull()`) or a
    /// tuple of them —
    /// is an initialisation: it reads no value, so a sibling scope can disturb nothing.
    fn is_constant(&self, val: &Value) -> bool {
        match val.unspan() {
            Value::Null
            | Value::Int(_)
            | Value::Enum(_, _)
            | Value::Boolean(_)
            | Value::Float(_)
            | Value::Long(_)
            | Value::Single(_)
            | Value::Text(_) => true,
            Value::Tuple(items) => items.iter().all(|i| self.is_constant(i)),
            Value::Call(op, args) => {
                let name = self.data.def(*op).name();
                args.is_empty() && name.starts_with("OpConv") && name.ends_with("FromNull")
            }
            _ => false,
        }
    }

    fn all(&mut self, list: &[Value]) {
        for n in list {
            self.node(n);
        }
    }

    /// Every form that names a variable counts as its use, not only `Var`: `Set`, `Iter`,
    /// `TupleGet`, `TuplePut`, `CallRef`, `FnRef` and `FnRefDnr` carry the number outside a
    /// `Var` node.  The forms with no arm here carry no variable number and no child node.
    fn node(&mut self, n: &Value) {
        match n {
            Value::Var(v) | Value::TupleGet(v, _) | Value::FnRefDnr(v) | Value::FnRef(_, v, _) => {
                self.use_of(*v);
            }
            Value::Set(v, val) => {
                self.node(val);
                if !self.is_constant(val) {
                    self.use_of(*v);
                }
            }
            Value::TuplePut(v, _, val) => {
                self.node(val);
                self.use_of(*v);
            }
            Value::Call(op, args)
                if self.releases.contains(op)
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(_))) =>
            {
                self.all(&args[1..]);
            }
            Value::CallRef(v, args) => {
                self.use_of(*v);
                self.all(args);
            }
            Value::Iter(v, source, next, body) => {
                self.use_of(*v);
                self.node(source);
                self.node(next);
                self.node(body);
            }
            Value::Block(bl) | Value::Loop(bl) => {
                self.open.push(bl.scope);
                self.all(&bl.operators);
                self.open.pop();
            }
            Value::If(c, t, e) => {
                self.node(c);
                // A pattern binds its names in the condition — a guard, a repetition's test —
                // and the arm reads them: the condition's blocks stay open in the THEN branch,
                // the arm the pattern binds into (`(B-Scope)`: a match arm's pattern binds into
                // its guard and its body).
                let mut cond = Vec::new();
                yield_scopes(c, &mut cond);
                let depth = self.open.len();
                self.open.extend(cond);
                self.node(t);
                self.open.truncate(depth);
                self.node(e);
            }
            Value::Call(_, args)
            | Value::Insert(args)
            | Value::Tuple(args)
            | Value::Parallel(args) => self.all(args),
            Value::Return(v) | Value::Drop(v) | Value::Yield(v) => self.node(v),
            Value::Span(s) => self.node(&s.1),
            Value::Null
            | Value::Line(_)
            | Value::Int(_)
            | Value::Enum(_, _)
            | Value::Boolean(_)
            | Value::Float(_)
            | Value::Long(_)
            | Value::Single(_)
            | Value::Text(_)
            | Value::Break(_)
            | Value::Continue(_)
            | Value::Keys(_)
            | Value::RawExpr(_) => {}
        }
    }
}

/// The scopes of the blocks `n` yields its value through: a block and its last operator, an
/// `if`'s two branches.  A variable these blocks bind is the condition's binding.
fn yield_scopes(n: &Value, out: &mut Vec<u16>) {
    match n.unspan() {
        Value::Block(bl) => {
            out.push(bl.scope);
            if let Some(last) = bl.operators.last() {
                yield_scopes(last, out);
            }
        }
        Value::If(_, t, e) => {
            yield_scopes(t, out);
            yield_scopes(e, out);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::check_function;
    use crate::data::{Block, Data, I32, Type, Value};
    use crate::variables::Function;

    fn block(scope: u16, operators: Vec<Value>) -> Value {
        Value::Block(Box::new(Block {
            name: "test",
            operators,
            result: Type::Void,
            scope,
            var_size: 0,
        }))
    }

    /// loft#2636's shape: `v` homed in the `if`'s first arm (scope 2), read after the `if` in
    /// the body (scope 1) — the slot is the next sibling's by then.
    #[test]
    fn a_local_homed_in_an_arm_and_read_after_it_is_reported() {
        let mut f = Function::new("n_t", "t.loft");
        let v = f.add_unique("v", &I32, 2);
        let code = block(
            1,
            vec![
                Value::If(
                    Box::new(Value::Boolean(true)),
                    Box::new(block(2, vec![Value::Set(v, Box::new(Value::Int(1)))])),
                    Box::new(block(3, vec![Value::Set(v, Box::new(Value::Int(2)))])),
                ),
                Value::Return(Box::new(Value::Var(v))),
            ],
        );
        let found = check_function(&f, &code, &[], &Data::new());
        assert_eq!(found.len(), 1, "one finding per variable");
        assert_eq!(
            (found[0].var, found[0].var_scope, found[0].use_scope),
            (v, 2, 1)
        );
    }

    /// The legal forms: a local of the enclosing block read in a nested one, a function-level
    /// local (scope 0), and a tuple read — another spelling of a use — inside the scope.
    #[test]
    fn uses_inside_the_scope_are_not_reported() {
        let mut f = Function::new("n_t", "t.loft");
        let outer = f.add_unique("o", &I32, 1);
        let top = f.add_unique("t", &I32, 0);
        let code = block(
            1,
            vec![
                Value::Set(outer, Box::new(Value::Int(1))),
                block(
                    2,
                    vec![
                        Value::Var(outer),
                        Value::TupleGet(outer, 0),
                        Value::Var(top),
                    ],
                ),
            ],
        );
        assert!(check_function(&f, &code, &[], &Data::new()).is_empty());
    }

    /// A use spelled outside a `Var` node is a use: a tuple read out of scope is reported.
    #[test]
    fn a_tuple_read_out_of_scope_is_reported() {
        let mut f = Function::new("n_t", "t.loft");
        let v = f.add_unique("v", &I32, 2);
        let code = block(1, vec![block(2, vec![]), Value::TupleGet(v, 0)]);
        assert_eq!(check_function(&f, &code, &[], &Data::new()).len(), 1);
    }
}
