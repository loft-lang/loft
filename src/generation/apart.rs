// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Header`'s apart clause — a vector the loop only READS, in a store nothing in the
//! loop is handed, keeps its header and its element base whatever else the loop does.
//!
//! The header gate (`hoist::blocks_header_hoist`) answers for a whole loop: one statement
//! that may write a store this analysis cannot type — a call of a function that grows the
//! buffer it is handed, a push whose root is not provably its own — and no vector of the
//! loop holds a header.  That is the right answer for a vector the statement can REACH, and
//! a cost for one it cannot: a store this activation minted, or a work buffer the caller
//! lends (`@FR-R-WorkBuffer`), is named by nothing but this frame's own variables, so a
//! statement that mentions none of them can neither move the vector nor grow its store.
//!
//! The mention test IS the proof.  Every variable whose dep chain ends at the store's
//! owner is in the store's alias set; the clause admits a root only where every mention of
//! that set in the loop body is a read in place — the vector as the operand of a length or
//! an element address, an element as the operand of a getter or a null test, or bound to a
//! local that borrows.  A mention this does not name TAINTS the store, and its vectors
//! take no header: the fallback costs the hoist, never a value.
use std::collections::HashSet;

use super::hoist::{PathKey, SCALAR_GETTERS, StoreFacts, is_element_address, one_op_wrapper};
use crate::data::{Block, Data, Value};

/// Where a node stands: its value dropped, read as a vector or through an element by the
/// operator above it, bound to a local of the same store, or anything else.
#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    Other,
    Discard,
    ReadVec,
    ReadElem,
    Flow(u16),
}

struct ApartWalk<'a, 'f> {
    data: &'a Data,
    facts: &'a StoreFacts<'f>,
    /// The store owners the candidates end at.
    owners: &'a HashSet<u16>,
    /// The owners some statement of the body names outside a read.
    tainted: HashSet<u16>,
}

impl ApartWalk<'_, '_> {
    /// The candidate owners whose store `x` may name: every end of its dep chain, each
    /// branch followed — a variable that borrows from two places names both.
    fn owners_of(&self, x: u16) -> Vec<u16> {
        let vars = self.facts.vars;
        let mut out = Vec::new();
        let mut seen: HashSet<u16> = HashSet::new();
        let mut stack = vec![x];
        while let Some(v) = stack.pop() {
            if v as usize >= vars.var_count() || !seen.insert(v) {
                continue;
            }
            if self.owners.contains(&v) && !out.contains(&v) {
                out.push(v);
            }
            stack.extend(vars.tp(v).depend().into_iter().filter(|d| *d != v));
        }
        out
    }

    /// The op a call stands for: a one-op stdlib wrapper over its first parameter reads as
    /// its op (`len(v)`), as the header emitter reads it.
    fn op_name(&self, d: u32) -> &str {
        use super::hoist::WrapperOperand;
        if let Some((op, operands)) = one_op_wrapper(self.data, d)
            && matches!(operands.first(), Some(WrapperOperand::Param(0)))
            && !operands
                .iter()
                .skip(1)
                .any(|o| matches!(o, WrapperOperand::Param(0)))
        {
            return self.data.def(op).name();
        }
        self.data.def(d).name()
    }

    fn list(&mut self, ops: &[Value], tail: Ctx) {
        for (i, op) in ops.iter().enumerate() {
            let ctx = if i + 1 == ops.len() {
                tail
            } else {
                Ctx::Discard
            };
            self.node(op, ctx);
        }
    }

    fn mention(&mut self, x: u16, ctx: Ctx) {
        // A text VALUE read out of the store is a `&str` or a `String` on native, never a
        // handle on the store (`@FR-R-TextBorrow`'s text-value clause): nothing writes or
        // moves a record through it.  A `&text` link is a `RefVar` and is not this.
        let vars = self.facts.vars;
        if (x as usize) < vars.var_count()
            && matches!(vars.tp(x).base(), crate::data::Type::Text(_))
        {
            return;
        }
        let named = self.owners_of(x);
        if named.is_empty() {
            return;
        }
        let admitted = match ctx {
            Ctx::ReadVec | Ctx::ReadElem | Ctx::Discard => true,
            // Bound to a local that borrows from every store this names: that local is in
            // their alias sets, and its own mentions are judged.
            Ctx::Flow(to) => {
                let holder = self.owners_of(to);
                !self.owners.contains(&to) && named.iter().all(|t| holder.contains(t))
            }
            Ctx::Other => false,
        };
        if !admitted {
            self.tainted.extend(named);
        }
    }

    fn node(&mut self, node: &Value, ctx: Ctx) {
        match node.unspan() {
            Value::Var(x) => self.mention(*x, ctx),
            Value::Call(d, args) if (*d as usize) < self.data.definitions.len() => {
                let name = self.op_name(*d);
                let first = if is_element_address(self.data, *d) && args.len() == 3 {
                    // The element's address is a view of the store: it may be read through
                    // or bound to a borrowing local, and nothing else.
                    match ctx {
                        Ctx::ReadElem | Ctx::Discard => Ctx::ReadVec,
                        // Bound to a local: the vector's mention is judged as that bind.
                        Ctx::Flow(_) => ctx,
                        _ => Ctx::Other,
                    }
                } else if matches!(name, "OpLengthVector" | "OpVectorIsNull") {
                    Ctx::ReadVec
                } else if SCALAR_GETTERS.contains(&name)
                    || matches!(name, "OpGetText" | "OpConvBoolFromRef")
                {
                    Ctx::ReadElem
                } else {
                    Ctx::Other
                };
                for (i, a) in args.iter().enumerate() {
                    self.node(a, if i == 0 { first } else { Ctx::Other });
                }
            }
            Value::Call(_, args) => {
                for a in args {
                    self.node(a, Ctx::Other);
                }
            }
            Value::Set(x, rhs) => {
                // The OWNER set inside the loop is its store minted, rebound or released:
                // whatever the header described on the way in is gone.
                if self.owners.contains(x) {
                    self.tainted.insert(*x);
                }
                // A null declares; anything else binds, and a bind of an alias-set local
                // from a view of its own store is that view's one admitted destination.
                if !matches!(rhs.unspan(), Value::Null) {
                    self.node(rhs, Ctx::Flow(*x));
                }
            }
            Value::Block(bl) => self.list(&bl.operators, ctx),
            Value::Loop(bl) => self.list(&bl.operators, Ctx::Discard),
            Value::Insert(xs) => self.list(xs, ctx),
            Value::If(c, t, e) => {
                self.node(c, Ctx::Other);
                self.node(t, ctx);
                self.node(e, ctx);
            }
            Value::Drop(e) => self.node(e, Ctx::Discard),
            Value::Return(e) | Value::Yield(e) => self.node(e, Ctx::Other),
            Value::Iter(x, a, b, c) => {
                self.mention(*x, Ctx::Other);
                self.node(a, Ctx::Other);
                self.node(b, Ctx::Other);
                self.node(c, Ctx::Other);
            }
            Value::TupleGet(base, _) => self.mention(*base, Ctx::Other),
            Value::TuplePut(base, _, inner) => {
                self.mention(*base, Ctx::Other);
                self.node(inner, Ctx::Other);
            }
            Value::CallRef(f, xs) => {
                self.mention(*f, Ctx::Other);
                for x in xs {
                    self.node(x, Ctx::Other);
                }
            }
            Value::FnRef(_, clos, _) => self.mention(*clos, Ctx::Other),
            Value::FnRefDnr(x) => self.mention(*x, Ctx::Other),
            Value::Tuple(xs) | Value::Parallel(xs) => {
                for x in xs {
                    self.node(x, Ctx::Other);
                }
            }
            // A leaf: every variant that carries a child or a variable number is named
            // above (the list `Value::for_each_child` and the var-carrying variants give).
            _ => {}
        }
    }
}

/// Of `candidates` — the vector paths `body` reads through an element address — the ones
/// the apart clause keeps: a bare variable whose store's owner is this activation's own
/// (`StoreFacts::fresh`) or a work buffer the caller lends (`lent`), and whose store the
/// body names only to read.
pub fn apart_headers(
    body: &Block,
    data: &Data,
    facts: &StoreFacts,
    lent: &dyn Fn(u16) -> bool,
    candidates: Vec<(PathKey, Value)>,
) -> Vec<(PathKey, Value)> {
    let owner_of = |p: &PathKey| {
        if !p.1.is_empty() {
            return None;
        }
        facts
            .terminal(p.0)
            .filter(|t| facts.owns_alone(*t) || lent(*t))
    };
    let mut owners: HashSet<u16> = candidates.iter().filter_map(|(p, _)| owner_of(p)).collect();
    // A closure reaches what it captured without being handed it, and a `&` link names a
    // store through a variable the loop may never mention: a store with a captured or a
    // linked name in its alias set is not this loop's alone.
    let vars = facts.vars;
    if !owners.is_empty() {
        let probe = ApartWalk {
            data,
            facts,
            owners: &owners,
            tainted: HashSet::new(),
        };
        let reached: Vec<u16> = (0..vars.var_count() as u16)
            .filter(|x| {
                vars.is_captured(*x) || matches!(vars.tp(*x).base(), crate::data::Type::RefVar(_))
            })
            .flat_map(|x| probe.owners_of(x))
            .collect();
        for t in reached {
            owners.remove(&t);
        }
    }
    // A suspended or forked body is not this loop running: a header bound before it would
    // be read by a later resumption, or beside a worker.
    if owners.is_empty()
        || body
            .operators
            .iter()
            .any(|op| op.any_node(&mut |n| matches!(n, Value::Yield(_) | Value::Parallel(_))))
    {
        return Vec::new();
    }
    let mut walk = ApartWalk {
        data,
        facts,
        owners: &owners,
        tainted: HashSet::new(),
    };
    walk.list(&body.operators, Ctx::Discard);
    let tainted = walk.tainted;
    candidates
        .into_iter()
        .filter(|(p, _)| owner_of(p).is_some_and(|t| owners.contains(&t) && !tainted.contains(&t)))
        .collect()
}
