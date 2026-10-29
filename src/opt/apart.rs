// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
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
//!
//! The PARAMETER clause: a parameter's store is named by the caller too, so the mention test
//! alone proves nothing about it — another argument, or the return buffer, may be a field of
//! the record it is.  A parameter's vector (a bare one or a field path over it) is kept only
//! when, beside the mention test, every variable the body names outside a read — every
//! store the body can write or grow — is proven apart from the parameter: a store this
//! activation minted (and that is no argument), or a pair `@FR-R-Alias`'s versioned copy
//! tested at run time.  A field projection read in place reads its base, and an argument
//! handed to a callee that writes no store and answers no reference is a read.
use std::collections::{HashMap, HashSet};

use super::hoist::{
    HoistOwned, PathKey, SCALAR_GETTERS, StoreFacts, is_element_address, one_op_wrapper,
};
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
    /// Every reference-typed variable the body names outside a read: the stores it can
    /// write or grow, which the parameter clause must find apart from its parameter.
    writers: HashSet<u16>,
    /// The per-callee "writes a store" memo, `hoist::call_writes_store`'s.
    cache: &'a mut HashMap<u32, bool>,
    owned: Option<&'a HoistOwned>,
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

    /// Is `d` a loft-bodied callee that writes no store and answers no reference — so what it
    /// is handed it can only read, and keeps nothing of?
    fn store_free_call(&mut self, d: u32) -> bool {
        let def = self.data.def(d);
        !matches!(def.code(), Value::Null)
            && !crate::data::is_dbref(def.returned())
            && !super::hoist::call_writes_store(
                d,
                self.data,
                self.cache,
                &mut HashSet::new(),
                self.owned,
            )
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
        // A text, and a SCALAR — a number, a boolean, a character, a plain enum's byte — names
        // no store, whatever its type's deps record about where it was read from (a `match` on
        // `op.code` carries the vector the op came from).
        if (x as usize) < vars.var_count()
            && matches!(
                vars.tp(x).base(),
                crate::data::Type::Text(_)
                    | crate::data::Type::Integer(_)
                    | crate::data::Type::Boolean
                    | crate::data::Type::Float
                    | crate::data::Type::Single
                    | crate::data::Type::Character
                    | crate::data::Type::Enum(_, false, _)
            )
        {
            return;
        }
        let named = self.owners_of(x);
        // A writer is anything that can name a store: a reference, a collection, and a `&`
        // link of ANY type (`x = &o.f` writes `o`'s store through it).
        if matches!(ctx, Ctx::Other)
            && (x as usize) < vars.var_count()
            && (crate::data::is_dbref(vars.tp(x).base())
                || matches!(vars.tp(x).base(), crate::data::Type::RefVar(_)))
        {
            self.writers.insert(x);
        }
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
                } else if name == "OpGetField" && args.len() == 3 {
                    // A field projection read in place reads its base; bound to a local, the
                    // local is the view and its own mentions are judged.
                    match ctx {
                        Ctx::ReadVec | Ctx::ReadElem | Ctx::Discard => Ctx::ReadElem,
                        Ctx::Flow(_) => ctx,
                        Ctx::Other => Ctx::Other,
                    }
                } else if self.store_free_call(*d) {
                    // Every argument of a callee that writes no store, and answers no
                    // reference into one, is read and nothing else.
                    for a in args {
                        self.node(a, Ctx::ReadElem);
                    }
                    return;
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

/// What the parameter clause asks beyond the loop: the per-callee "writes a store" memo, the
/// emitter-owned buffers that memo exempts, and the whole function's code (for the locals
/// only ever minted).
pub struct Callees<'a> {
    pub cache: &'a mut HashMap<u32, bool>,
    pub owned: Option<&'a HoistOwned>,
    pub code: &'a Value,
}

struct MintOps {
    database: u32,
    database_np: u32,
}

/// Is every store `w` names one this activation minted?  A local that is no argument, borrows
/// from nothing, is bound to nothing but a null, and is minted (`OpDatabase` / `OpDatabaseNP`
/// with it as the store) — each mint a store that did not exist before, so no parameter's.
fn minted_here(code: &Value, vars: &crate::variables::Function, w: u16, ops: &MintOps) -> bool {
    // @FR-O-Proxy asks alloc — an empty dep list beside a mint at every binding is a store
    // this call allocated; it drives a header hold, never a free.
    if (w as usize) >= vars.var_count() || vars.is_argument(w) || !vars.tp(w).depend().is_empty() {
        return false;
    }
    let mut minted = false;
    let mut other = false;
    code.walk(&mut |n| match n {
        Value::Set(x, rhs) if *x == w && !matches!(rhs.unspan(), Value::Null) => other = true,
        Value::Call(d, args)
            if (*d == ops.database || *d == ops.database_np)
                && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == w) =>
        {
            minted = true;
        }
        _ => {}
    });
    minted && !other
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
    callees: Callees<'_>,
) -> Vec<(PathKey, Value)> {
    let Callees { cache, owned, code } = callees;
    let vars = facts.vars;
    let ops = MintOps {
        database: data.def_nr("OpDatabase"),
        database_np: data.def_nr("OpDatabaseNP"),
    };
    // A parameter the clause may consider: a by-value record or vector argument, neither a
    // `&` link nor captured (those name the store through what the loop may never mention).
    let param = |t: u16| {
        crate::keys::apart_param_enabled()
            && vars.is_argument(t)
            && !vars.is_captured(t)
            && !matches!(vars.tp(t).base(), crate::data::Type::RefVar(_))
            && crate::data::is_dbref(vars.tp(t).base())
    };
    let owner_of = |p: &PathKey| {
        let t = facts.terminal(p.0)?;
        if p.1.is_empty() && (facts.owns_alone(t) || lent(t)) {
            return Some(t);
        }
        param(t).then_some(t)
    };
    let mut owners: HashSet<u16> = candidates.iter().filter_map(|(p, _)| owner_of(p)).collect();
    // A closure reaches what it captured without being handed it, and a `&` link names a
    // store through a variable the loop may never mention: a store with a captured or a
    // linked name in its alias set is not this loop's alone.
    if !owners.is_empty() {
        let mut probe_cache = HashMap::new();
        let probe = ApartWalk {
            data,
            facts,
            owners: &owners,
            tainted: HashSet::new(),
            writers: HashSet::new(),
            cache: &mut probe_cache,
            owned,
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
        writers: HashSet::new(),
        cache,
        owned,
    };
    walk.list(&body.operators, Ctx::Discard);
    let ApartWalk {
        mut tainted,
        writers,
        ..
    } = walk;
    // The parameter clause: every store the body can write or grow is proven apart from the
    // parameter's.  A writer in the parameter's own alias set has tainted it already.
    for &t in &owners {
        if param(t)
            && !facts.owns_alone(t)
            && !writers.iter().all(|&w| {
                facts.terminal(w) == Some(t)
                    || facts.apart_from_argument(t, w)
                    || minted_here(code, vars, w, &ops)
            })
        {
            tainted.insert(t);
        }
    }
    let kept: Vec<(PathKey, Value)> = candidates
        .into_iter()
        .filter(|(p, _)| owner_of(p).is_some_and(|t| owners.contains(&t) && !tainted.contains(&t)))
        .collect();
    crate::rewrite_census::fired(
        "R-Header/apart-param",
        kept.iter()
            .filter(|(p, _)| owner_of(p).is_some_and(|t| param(t) && !facts.owns_alone(t)))
            .count(),
    );
    kept
}
