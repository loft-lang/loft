// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-Base`'s guard clause — the element accesses a passed test keeps below their
//! vector's length.
//!
//! `v[e]` reads or writes an element whose index is below `len(v)` when a test `e < len(v)`
//! passed on every path to it and nothing evaluated since could resize `v` or assign a
//! variable `e` names.  The test is an `if`'s condition (in force in its then-arm), a `while`'s
//! (in force from the loop's second statement on, in every turn: the next turn tests again), or
//! either side of `a && b`.  The bound is `len(v)` or a local holding it
//! (`length_locals::own_length_locals`).  `v` is the frame's own vector (`own_vector`): a
//! parameter can share its vector with another name, which a write would shorten unseen.
//!
//! What ends a fact is `forward_walk::check_body`'s answer for `v` and each variable of `e` —
//! an assignment of either, `v` handed to an operator that may resize it, a call through a
//! reference, a yield, a par — applied in evaluation order, and over a nested loop's whole body
//! before the loop is entered (a later turn runs a kill before an earlier read).  Element writes
//! resize nothing.
//!
//! The answer is the upper bound only: a writer that drops the index test also needs `e >= 0`
//! (`(R-Range)`'s index clause), since loft reads a negative index from the end.
//! `LOFT_NO_GUARD_BOUND=1` answers none.

use crate::data::{Data, Value};
use crate::forward_walk::{Walk, check_body};
use crate::variables::Function;
use std::collections::HashSet;

/// A test that passed: `e < len(v)`.
#[derive(Clone)]
struct Fact {
    v: u16,
    e: Value,
    vars: Vec<u16>,
}

struct Cx<'a> {
    data: &'a Data,
    vars: &'a Function,
    attrs: &'a [crate::data::Attribute],
    own_len: Vec<(u16, u16)>,
    out: HashSet<usize>,
}

/// The index nodes of function `d` (by address, the `e` of each `OpGetVector*(v, size, e)`)
/// that a passed test keeps below their vector's length.
#[must_use]
pub fn guarded_indices(data: &Data, d: u32) -> HashSet<usize> {
    if crate::env_once!(std::env::var("LOFT_NO_GUARD_BOUND").is_ok_and(|v| v != "0")) {
        return HashSet::new();
    }
    let def = data.def(d);
    let vars = def.variables();
    let mut cx = Cx {
        data,
        vars,
        attrs: def.attributes(),
        own_len: crate::parser::length_locals::own_length_locals(data, d, vars),
        out: HashSet::new(),
    };
    cx.visit(def.code(), &mut Vec::new());
    cx.out
}

fn name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

/// The same index expression: a variable, a constant, or a pure integer operator over such.
fn same(a: &Value, b: &Value) -> bool {
    match (a.unspan(), b.unspan()) {
        (Value::Var(x), Value::Var(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Long(x), Value::Long(y)) => x == y,
        (Value::Call(f, xa), Value::Call(g, ya)) => {
            f == g && xa.len() == ya.len() && xa.iter().zip(ya).all(|(x, y)| same(x, y))
        }
        _ => false,
    }
}

impl Cx<'_> {
    /// An index expression a fact can carry: variables and constants under `+`, `-`, `*` —
    /// the same value wherever it is evaluated while its variables keep theirs.  Its variables.
    fn pure(&self, e: &Value, vars: &mut Vec<u16>) -> bool {
        match e.unspan() {
            Value::Var(x) => {
                if !vars.contains(x) {
                    vars.push(*x);
                }
                true
            }
            Value::Int(_) | Value::Long(_) => true,
            Value::Call(d, a) => {
                matches!(name(self.data, *d), "OpAddInt" | "OpSubInt" | "OpMulInt")
                    && a.iter().all(|x| self.pure(x, vars))
            }
            _ => false,
        }
    }

    /// The vector `l` is the length of: `len(v)`, or a local holding it.
    fn length_of(&self, l: &Value) -> Option<u16> {
        match l.unspan() {
            Value::Call(d, a)
                if a.len() == 1 && matches!(name(self.data, *d), "OpLengthVector" | "t_6vector_len") =>
            {
                let Value::Var(v) = a[0].unspan() else {
                    return None;
                };
                crate::parser::length_locals::own_vector(self.attrs, self.vars, *v).then_some(*v)
            }
            Value::Var(n) => self.own_len.iter().find(|(m, _)| m == n).map(|(_, v)| *v),
            _ => None,
        }
    }

    /// The facts a passed test `test` establishes.
    fn facts(&self, test: &Value, into: &mut Vec<Fact>) {
        match test.unspan() {
            // `left && right`
            Value::If(left, right, other) if matches!(other.unspan(), Value::Boolean(false)) => {
                self.facts(left, into);
                self.facts(right, into);
            }
            Value::Call(op, args) if args.len() == 2 => {
                let (index, bound) = match name(self.data, *op) {
                    "OpLtInt" => (&args[0], &args[1]),
                    "OpGtInt" => (&args[1], &args[0]),
                    _ => return,
                };
                let mut vars = Vec::new();
                if let Some(v) = self.length_of(bound)
                    && self.pure(index, &mut vars)
                {
                    into.push(Fact {
                        v,
                        e: index.clone(),
                        vars,
                    });
                }
            }
            _ => {}
        }
    }

    /// Forget every fact `node` may end.
    fn kill(&self, live: &mut Vec<Fact>, node: &Value) {
        live.retain(|f| {
            let idxs: &[u16] = if f.vars.is_empty() { &[u16::MAX] } else { &f.vars };
            idxs.iter().all(|&x| {
                let w = Walk {
                    vec_t: f.v,
                    src: f.v,
                    idx: x,
                    lv: x,
                    read: Value::Null,
                    iter_scope: 0,
                    field: None,
                };
                check_body(self.data, self.vars, &w, node).is_ok()
            })
        });
    }

    fn visit(&mut self, node: &Value, live: &mut Vec<Fact>) {
        match node {
            Value::Span(s) => self.visit(&s.1, live),
            Value::If(c, t, e) => {
                self.visit(c, live);
                self.kill(live, c);
                let mut then_facts = live.clone();
                self.facts(c, &mut then_facts);
                self.visit(t, &mut then_facts);
                let mut else_facts = live.clone();
                self.visit(e, &mut else_facts);
                self.kill(live, node);
            }
            Value::Loop(lp) => {
                self.kill(live, node);
                let mut inner = live.clone();
                let mut ops = lp.operators.iter().peekable();
                while let Some(op) = ops.next_if(|o| matches!(o.unspan(), Value::Line(_))) {
                    self.visit(op, &mut inner);
                }
                // `while c { … }`: the loop's first statement is `if !c break`.
                if let Some(first) = ops.next() {
                    self.visit(first, &mut inner);
                    self.kill(&mut inner, first);
                    if let Value::If(c, _, _) = first.unspan()
                        && let Value::Call(d, a) = c.unspan()
                        && a.len() == 1
                        && name(self.data, *d) == "OpNot"
                    {
                        self.facts(&a[0], &mut inner);
                    }
                }
                for op in ops {
                    self.visit(op, &mut inner);
                    self.kill(&mut inner, op);
                }
            }
            Value::Block(b) => {
                for op in &b.operators {
                    self.visit(op, live);
                    self.kill(live, op);
                }
            }
            Value::Call(d, args) => {
                if args.len() == 3
                    && matches!(name(self.data, *d), "OpGetVector" | "OpGetVectorNullable")
                    && let Value::Var(v) = args[0].unspan()
                    && live.iter().any(|f| f.v == *v && same(&f.e, &args[2]))
                {
                    self.out.insert(std::ptr::from_ref(&args[2]) as usize);
                }
                for a in args {
                    self.visit(a, live);
                    if !matches!(a.unspan(), Value::Var(_)) {
                        self.kill(live, a);
                    }
                }
            }
            other => {
                other.for_each_child(&mut |c| {
                    self.visit(c, live);
                    self.kill(live, c);
                });
            }
        }
    }
}
