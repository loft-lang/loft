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
//! A block whose last operator is a variable yields that variable as its value: the block's end
//! hands the value on instead of releasing it, so the variable is in scope wherever the block's
//! value is read after it (a `return` of a tuple a block built).
//!
//! For the same reason a variable that holds no store is not checked: an integer, a float, a
//! boolean, a character, a plain enum or a tuple of them.  Its scope's end releases nothing,
//! and its slot is live to its last mention, so a use after that end reads what the variable
//! holds.  Measured on loft#2636's shape with its fix reversed: the integer and text locals
//! answered right, the vector local answered empty.
//!
//! **Independence.**  The check reads only the IR's block nesting (`Block::scope`) and the
//! variable table's `scope`, never a predicate of `src/scopes/`, the pass whose result it
//! checks.  It runs under `LOFT_OWN_ORACLE=check-records`, the mode `scripts/own_sweep.py`
//! scores, and in the lifetime report (`LOFT_LIFETIME_CHECK`); it changes nothing it reads.

use crate::data::{Data, DefType, Type, Value};
use crate::variables::Function;

/// One use of a variable outside its scope.
pub struct Finding {
    pub var: u16,
    pub var_scope: u16,
    pub use_scope: u16,
    /// The source line of the statement holding the use (the IR's last `Line` marker), 0 when
    /// none precedes it.
    pub line: u32,
}

impl Finding {
    /// The one-line reason the report prints.
    #[must_use]
    pub fn reason(&self, vars: &Function) -> String {
        format!(
            "`{}` is used outside its scope: declared in scope {}, used in scope {}, after that \
             scope's end released its value",
            vars.name(self.var),
            self.var_scope,
            self.use_scope
        )
    }
}

/// Every use outside its scope in the closed program, `(definition, finding)`, in definition
/// order — for the one-report lifetime check (`crate::lifetime_check`).
pub fn findings(data: &Data) -> Vec<(u32, Finding)> {
    let releases = release_ops(data);
    let mut out = Vec::new();
    for d in 0..data.definitions() {
        let def = data.def(d);
        if !matches!(def.def_type, DefType::Function) || matches!(def.code.unspan(), Value::Null) {
            continue;
        }
        out.extend(
            check_function(&def.variables, &def.code, &releases, data)
                .into_iter()
                .map(|f| (d, f)),
        );
    }
    out
}

/// The `OpFree*` operators: a release of a variable reads no value of it.
fn release_ops(data: &Data) -> Vec<u32> {
    (0..data.definitions())
        .filter(|&d| data.def(d).name().starts_with("OpFree"))
        .collect()
}

/// The observer entry point, at the end of `close_program`.
pub fn observe(data: &Data) {
    let releases = release_ops(data);
    let Ok(mode) = std::env::var("LOFT_OWN_ORACLE") else {
        return;
    };
    if mode != "check-records" {
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
        yielded: Vec::new(),
        line: 0,
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
    /// The variables a block yielded as its value: they outlive that block.
    yielded: Vec<u16>,
    /// The source line of the statement being read.
    line: u32,
    /// The scopes enclosing the node being read, outermost first: the function's own (0), then
    /// one per enclosing block.
    open: Vec<u16>,
    found: Vec<Finding>,
}

impl Walk<'_> {
    /// A use of variable `v` here.  An argument lives for the whole call; a variable whose
    /// scope was never set (`u16::MAX`) was never placed, so it has no slot to lose.
    fn use_of(&mut self, v: u16) {
        if self.vars.is_argument(v) || holds_no_store(self.vars.tp(v)) || self.yielded.contains(&v)
        {
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
            line: self.line,
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
                if let Some(Value::Var(w)) = bl.operators.last().map(Value::unspan) {
                    self.yielded.push(*w);
                }
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
            Value::Line(l) => self.line = *l,
            Value::Null
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

/// A type whose value has no store, so a scope's end releases nothing of it.
fn holds_no_store(tp: &Type) -> bool {
    // `.base()`: a `τ?` is `τ`'s storage behind a nullability marker (`@FR-N-Shape`).
    match tp.base() {
        Type::Integer(_)
        | Type::Boolean
        | Type::Float
        | Type::Single
        | Type::Character
        | Type::Enum(_, false, _) => true,
        Type::Tuple(items) => items.iter().all(holds_no_store),
        _ => false,
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
    use crate::data::{Block, Data, Deps, I32, Type, Value};
    use crate::variables::Function;

    /// The definition number the tests use for a release (`OpFree*`).
    const FREE: u32 = 7;

    fn vec_tp() -> Type {
        Type::Vector(Box::new(I32.clone()), Deps::none())
    }

    fn block(scope: u16, operators: Vec<Value>) -> Value {
        Value::Block(Box::new(Block {
            name: "test",
            operators,
            result: Type::Void,
            scope,
            var_size: 0,
        }))
    }

    fn free(v: u16) -> Value {
        Value::Call(FREE, vec![Value::Var(v)])
    }

    fn check(f: &Function, code: &Value) -> Vec<(u16, u16, u16)> {
        check_function(f, code, &[FREE], &Data::new())
            .iter()
            .map(|x| (x.var, x.var_scope, x.use_scope))
            .collect()
    }

    /// loft#2636's shape: `v` homed in the `if`'s first arm (scope 2) and released at its end,
    /// then read after the `if` in the body (scope 1).
    #[test]
    fn a_local_homed_in_an_arm_and_read_after_it_is_reported() {
        let mut f = Function::new("n_t", "t.loft");
        let v = f.add_unique("v", &vec_tp(), 2);
        let code = block(
            1,
            vec![
                Value::If(
                    Box::new(Value::Boolean(true)),
                    Box::new(block(
                        2,
                        vec![Value::Set(v, Box::new(Value::Var(v))), free(v)],
                    )),
                    Box::new(block(3, vec![])),
                ),
                Value::Return(Box::new(Value::Var(v))),
            ],
        );
        assert_eq!(check(&f, &code), vec![(v, 2, 1)]);
    }

    /// The forms that are not reported: a local of the enclosing block read in a nested one, a
    /// function-level local (scope 0), an initialisation to a constant and a release outside the
    /// scope, a pattern's binder read in the THEN branch of the `if` whose condition bound it,
    /// and a variable that holds no store.
    #[test]
    fn uses_the_rule_allows_are_not_reported() {
        let mut f = Function::new("n_t", "t.loft");
        let outer = f.add_unique("o", &vec_tp(), 1);
        let top = f.add_unique("t", &vec_tp(), 0);
        let late = f.add_unique("l", &vec_tp(), 3);
        let bound = f.add_unique("b", &vec_tp(), 4);
        let kept = f.add_unique("k", &I32, 5);
        let code = block(
            1,
            vec![
                Value::Set(late, Box::new(Value::Null)),
                Value::Set(outer, Box::new(Value::Var(top))),
                block(2, vec![Value::Var(outer), Value::Var(top)]),
                block(3, vec![Value::Set(late, Box::new(Value::Var(top)))]),
                free(late),
                Value::If(
                    Box::new(block(4, vec![Value::Set(bound, Box::new(Value::Var(top)))])),
                    Box::new(Value::Var(bound)),
                    Box::new(Value::Null),
                ),
                block(5, vec![Value::Set(kept, Box::new(Value::Var(top)))]),
                Value::Var(kept),
                free(outer),
                free(top),
                free(bound),
            ],
        );
        assert_eq!(check(&f, &code), vec![]);
    }

    /// A use spelled outside a `Var` node is a use: a tuple read out of scope is reported.
    #[test]
    fn a_tuple_read_out_of_scope_is_reported() {
        let mut f = Function::new("n_t", "t.loft");
        let v = f.add_unique("v", &vec_tp(), 2);
        let code = block(1, vec![block(2, vec![free(v)]), Value::TupleGet(v, 0)]);
        assert_eq!(check(&f, &code), vec![(v, 2, 1)]);
    }

    /// A block that yields its variable hands it on: a `return` of it after the block reads
    /// the block's value, not a released one.
    #[test]
    fn a_block_s_yielded_variable_outlives_the_block() {
        let mut f = Function::new("n_t", "t.loft");
        let w = f.add_unique("w", &vec_tp(), 2);
        let code = block(
            1,
            vec![
                block(2, vec![Value::Set(w, Box::new(Value::Null)), Value::Var(w)]),
                Value::Return(Box::new(Value::Var(w))),
            ],
        );
        assert_eq!(check(&f, &code), vec![]);
    }

    /// The binder of an `if`'s condition is not in scope in its ELSE branch.
    #[test]
    fn a_condition_binder_read_in_the_else_branch_is_reported() {
        let mut f = Function::new("n_t", "t.loft");
        let v = f.add_unique("v", &vec_tp(), 2);
        let code = block(
            1,
            vec![Value::If(
                Box::new(block(
                    2,
                    vec![Value::Set(v, Box::new(Value::Null)), free(v)],
                )),
                Box::new(Value::Null),
                Box::new(Value::Var(v)),
            )],
        );
        assert_eq!(check(&f, &code), vec![(v, 2, 1)]);
    }
}
