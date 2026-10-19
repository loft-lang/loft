// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! `@FR-R-DecodeView`'s structure clause — the CONSUMERS: the functions of a root's chain that
//! hold a decoded value as a position (`decode_structure`).  The scan admits a closed list of
//! mentions of such a value and declines on anything else:
//!
//! - an assignment from a position source: a consumer's or the producer's call, a field of
//!   a decoded record or of an element, a unit literal, the decoder's own record literal, a
//!   vector field read inside the arm that proved its variant;
//! - a read the encoding answers: the enum tag, a text field inside the arm that proved its
//!   variant, a scalar field of the decoded record, a hand-off to a consumer;
//! - a walk `for e in v.entries { … }` in order from the first element, whose body reads no
//!   index and leaves only by `return`;
//! - a `return` of a position, and the frees, clears and buffer mints of position locals,
//!   which hold nothing and are dropped;
//! - (6) in instance 1: no position made outside a loop or walk is read inside it, and no
//!   consumer reaches itself.
use super::decode_structure::{
    Producer, PsPlan, addr, op_name, record_tp, unit_literal, unit_return,
};
use super::decode_view::{DvPlan, int_of};
use crate::data::{Block, Data, Type, Value};
use crate::database::{Parts, Stores};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};

fn trace(msg: impl FnOnce() -> String) {
    if std::env::var("LOFT_TRACE_DECODE_STRUCTURE").is_ok_and(|v| v != "0") {
        eprintln!("[decode-structure] {}", msg());
    }
}

/// What a consumer's variable holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rep {
    /// A value of the enum: a position or a unit literal.
    T,
    /// The producer's record: `(position, scalar fields…)`.
    D,
    /// An element record of a walked vector: a tuple of positions.
    E(u16),
    /// A vector field of a value, held as its owner's position; only walked.
    V(u16),
}

/// A walk of a vector field: the `for` block's element variable and its owner.
#[derive(Clone, Debug)]
pub struct Walk {
    pub elem: u16,
    /// The vector variable the loop reads (`_vector_1`).
    pub vec: u16,
    /// The walk twin's key: the field's offset and vector store type.
    pub field: (i64, u16),
    /// Index of the first body statement in the loop block.
    pub body_from: usize,
}

/// One consumer's plan.
#[derive(Clone, Debug, Default)]
pub struct Consumer {
    pub producer: u32,
    pub reps: HashMap<u16, Rep>,
    /// Statements dropped (frees, clears, buffer mints of positions).
    pub skip: HashSet<usize>,
    /// `For` blocks (IR address) → their walk.
    pub walks: HashMap<usize, Walk>,
    /// The hidden buffer parameter of a function answering a position: no parameter here.
    pub retbuf: Option<u16>,
    /// The function answers a position (`T` or `D`).
    pub returns: Option<Rep>,
    /// The frame variable, when the function has one.
    pub frame: Option<u16>,
    /// The consumers it hands a position to.
    pub callees: Vec<u32>,
}

struct CScan<'a> {
    data: &'a Data,
    stores: &'a Stores,
    plan: &'a PsPlan,
    p: &'a Producer,
    pd: u32,
    d: u32,
    c: Consumer,
    /// Each position variable's loop depth at its assignment.
    depth_of: HashMap<u16, usize>,
    depth: usize,
    /// The variants the enclosing arms proved: `(value expression, tag)`.
    arms: Vec<(&'a Value, u8)>,
    /// Consumers this one hands a position to.
    callees: Vec<u32>,
    why: Option<String>,
    /// The walk being scanned answers through its closure.
    in_walk: usize,
    /// How often each position — a local, or a field of one — is handed on: walked, copied,
    /// handed to a consumer or returned.
    uses: HashMap<(u16, i64), usize>,
}

impl<'a> CScan<'a> {
    fn fail(&mut self, why: impl FnOnce() -> String) {
        if self.why.is_none() {
            self.why = Some(why());
        }
    }

    fn name(&self, v: u16) -> String {
        self.data.def(self.d).variables().name(v).to_string()
    }

    fn rep(&self, v: u16) -> Option<Rep> {
        self.c.reps.get(&v).copied()
    }

    fn read(&mut self, v: u16) {
        if let Some(at) = self.depth_of.get(&v).copied()
            && at < self.depth
        {
            let n = self.name(v);
            self.fail(|| format!("`{n}` is read inside a loop it was not made in"));
        }
    }

    /// (6) READ ONCE: the position `expr` names is handed on — walked, copied, handed to a
    /// consumer or returned — here; a second hand-off is a second walk of the same encoding.
    /// A tag test reads only the head, and a field read inside the arm that proved the variant
    /// is part of that match: neither counts.
    fn hand_on(&mut self, expr: &Value) {
        let key = match expr.unspan() {
            Value::Var(var) => (*var, -1),
            Value::Call(op, args) if op_name(self.data, *op) == "OpGetField" && args.len() == 3 => {
                let (Value::Var(var), Some(off)) = (args[0].unspan(), int_of(&args[1])) else {
                    return;
                };
                (*var, off)
            }
            _ => return,
        };
        let count = self.uses.entry(key).or_default();
        *count += 1;
        if *count > 1 {
            let n = self.name(key.0);
            self.fail(|| format!("`{n}` is handed on twice: read once (6) does not hold"));
        }
    }

    /// A position expression: answers its kind.
    fn t_expr(&mut self, v: &'a Value) -> Option<Rep> {
        match v.unspan() {
            Value::Var(expr) => {
                let r = self.rep(*expr)?;
                self.read(*expr);
                Some(r)
            }
            Value::Call(callee, args)
                if op_name(self.data, *callee) == "OpGetField" && args.len() == 3 =>
            {
                let Value::Var(expr) = args[0].unspan() else {
                    return None;
                };
                let off = int_of(&args[1])?;
                match self.rep(*expr)? {
                    Rep::D if off == self.p.vo => {
                        self.read(*expr);
                        Some(Rep::T)
                    }
                    Rep::E(etp) => {
                        self.read(*expr);
                        let fs = elem_fields(self.stores, etp)?;
                        fs.iter()
                            .any(|f| i64::from(f.position) == off)
                            .then_some(Rep::T)
                    }
                    Rep::T => {
                        let vtp = u16::try_from(int_of(&args[2])?).ok()?;
                        if !self.p.elem.contains_key(&vtp)
                            || !self.p.walked.contains_key(&(off, vtp))
                        {
                            return None;
                        }
                        self.proven(&args[0], |stores, tag_fields| {
                            tag_fields
                                .iter()
                                .any(|(pos, content)| *pos == off && *content == vtp)
                                && stores.types.len() > vtp as usize
                        })?;
                        self.read(*expr);
                        Some(Rep::V(vtp))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The enclosing arm proved `x`'s variant, and that variant's fields pass `test`.
    fn proven(&self, expr: &Value, test: impl Fn(&Stores, &[(i64, u16)]) -> bool) -> Option<()> {
        let (_, tag) = self
            .arms
            .iter()
            .rev()
            .find(|(e, _)| e.unspan() == expr.unspan())?;
        let fs = variant_fields(self.stores, self.p.t_tp, *tag)?;
        test(self.stores, &fs).then_some(())
    }

    /// An expression mentioning positions in plain code.
    fn expr(&mut self, v: &'a Value) {
        if self.why.is_some() {
            return;
        }
        match v.unspan() {
            Value::Var(expr) => {
                if self.rep(*expr).is_some() {
                    let n = self.name(*expr);
                    self.fail(|| format!("`{n}` is used whole"));
                }
            }
            Value::Call(callee, args) => self.call(*callee, args),
            Value::Set(expr, rhs) => self.set(v, *expr, rhs),
            Value::Block(bl) => self.block(v, bl),
            Value::Insert(items) => {
                for o in items {
                    self.expr(o);
                }
            }
            Value::If(c, t, e) => self.if_(v, c, t, e),
            Value::Loop(bl) => {
                self.depth += 1;
                for o in &bl.operators {
                    self.expr(o);
                }
                self.depth -= 1;
            }
            Value::Return(r) => self.ret(r),
            Value::Drop(expr) => self.expr(expr),
            Value::Yield(_) | Value::CallRef(..) | Value::FnRef(..) => {
                if super::decode_structure::mentions_any(v, &self.c.reps.keys().copied().collect())
                {
                    self.fail(|| "a position reaches a yield or a function reference".into());
                }
            }
            Value::Break(_) | Value::Continue(_) => {
                if self.in_walk > 0 && self.depth == 0 {
                    self.fail(|| "a walk is left by a break or a continue".into());
                }
            }
            _ => {
                if super::decode_structure::mentions_any(v, &self.c.reps.keys().copied().collect())
                {
                    self.fail(|| "a position in an unknown form".into());
                }
            }
        }
    }

    fn call(&mut self, callee: u32, args: &'a [Value]) {
        let name = op_name(self.data, callee);
        if self.position_read(name, args) {
            return;
        }
        if name.starts_with("Op") {
            // An operator over plain values: what it reads of a position was matched above,
            // and any other mention declines in its operand.
            for expr in args {
                self.expr(expr);
            }
            return;
        }
        if callee == self.pd {
            // The producer's result: a decoded record, held as the call's position.
            self.producer_call(args);
            return;
        }
        if self.plan.producers.contains_key(&callee) {
            self.fail(|| "a second producer".into());
            return;
        }
        let def = self.data.def(callee);
        let hands = args.iter().any(|expr| self.mentions_rep(expr));
        let answers = self.rep_type(def.returned()).is_some();
        if hands || answers {
            self.consumer_call(callee, args, answers);
            return;
        }
        for expr in args {
            self.expr(expr);
        }
    }

    /// A read the encoding answers, or a release of a position.  Answers whether `args` was
    /// one (and checked).
    fn position_read(&mut self, name: &str, args: &'a [Value]) -> bool {
        match (name, args) {
            ("OpGetBoolean" | "OpGetInt", [Value::Var(rec), off])
                if self.rep(*rec) == Some(Rep::D) =>
            {
                if int_of(off).is_none_or(|o| self.p.scalar_index(o).is_none()) {
                    self.fail(|| "a field of the decoded record that is no scalar".into());
                }
                self.read(*rec);
                true
            }
            ("OpConvIntFromEnum", [inner]) => {
                matches!(inner.unspan(), Value::Call(get, get_args)
                    if op_name(self.data, *get) == "OpGetEnum"
                        && get_args.len() == 2
                        && int_of(&get_args[1]) == Some(0)
                        && self.t_expr(&get_args[0]) == Some(Rep::T))
            }
            ("OpGetEnum", [expr, zero]) => {
                int_of(zero) == Some(0) && self.t_expr(expr) == Some(Rep::T)
            }
            // A number, boolean or text field of a value: the shape's slot, inside the arm
            // that proved the variant.
            ("OpGetInt" | "OpGetBoolean" | "OpGetText", [expr, off])
                if self.t_expr(expr) == Some(Rep::T) =>
            {
                let want = match name {
                    "OpGetInt" => self.stores.name("integer"),
                    "OpGetBoolean" => self.stores.name("boolean"),
                    _ => self.stores.name("text"),
                };
                let proven = int_of(off).and_then(|off| {
                    self.proven(expr, |_, fields| {
                        fields
                            .iter()
                            .any(|(pos, content)| *pos == off && *content == want)
                    })
                });
                if proven.is_none() {
                    self.fail(|| "a field read outside the arm that proved its variant".into());
                }
                true
            }
            // Releases, clears and buffer mints of positions: nothing to release.
            (
                "OpFreeRef"
                | "OpFreeRefIfDistinct"
                | "OpClear"
                | "OpDatabase"
                | "OpDatabaseNP"
                | "OpRefIsNull"
                | "OpConvBoolFromRef"
                | "OpDistinctStore"
                | "OpRefAlias",
                _,
            ) => {
                args.iter().any(
                    |expr| matches!(expr.unspan(), Value::Var(var) if self.rep(*var).is_some()),
                ) && args.iter().all(|expr| {
                    matches!(
                        expr.unspan(),
                        Value::Var(_) | Value::Int(_) | Value::Long(_)
                    )
                })
            }
            _ => false,
        }
    }

    /// A position handed to, or answered by, another consumer.
    fn consumer_call(&mut self, callee: u32, args: &'a [Value], answers: bool) {
        let def = self.data.def(callee);
        if !matches!(def.def_type(), crate::data::DefType::Function)
            || matches!(def.code(), Value::Null)
        {
            let n = def.name().to_string();
            self.fail(|| format!("a position reaches `{n}`, which is no loft function"));
            return;
        }
        if callee == self.d {
            self.fail(|| "a consumer reaches itself".into());
            return;
        }
        self.callees.push(callee);
        let rb = if answers {
            super::hoist::ret_buffer_attr(def)
        } else {
            None
        };
        for (i, expr) in args.iter().enumerate() {
            if Some(i) == rb {
                continue;
            }
            let param = def.attributes().get(i).map(|attr| &attr.typedef);
            let Some(want) = param.and_then(|tp| self.rep_type(tp)) else {
                self.expr(expr);
                continue;
            };
            self.hand_on(expr);
            let got = match expr.unspan() {
                Value::Var(var) => {
                    let rep = self.rep(*var);
                    if rep.is_some() {
                        self.read(*var);
                    }
                    rep
                }
                _ => self.t_expr(expr),
            };
            if got != Some(want) {
                self.fail(|| {
                    "a call is handed something other than a position where it takes one".into()
                });
            }
        }
    }

    fn producer_call(&mut self, args: &'a [Value]) {
        let def = self.data.def(self.pd);
        for (i, expr) in args.iter().enumerate() {
            let pv = def.variables().var(&def.attributes()[i].name);
            if pv == self.p.frame {
                let ok = matches!(expr.unspan(), Value::Var(y) if self.c.frame == Some(*y));
                if !ok {
                    self.fail(|| "the producer is handed another frame".into());
                }
            } else if pv == self.p.retbuf {
                // The buffer: a position local, or anything that names no position.
            } else {
                self.expr(expr);
            }
        }
    }

    fn mentions_rep(&self, expr: &Value) -> bool {
        expr.any_node(&mut |n| matches!(n, Value::Var(y) if self.c.reps.contains_key(y)))
    }

    fn rep_type(&self, tp: &Type) -> Option<Rep> {
        rep_of(self.data, self.p, tp)
    }

    fn set(&mut self, v: &'a Value, expr: u16, rhs: &'a Value) {
        let Some(want) = self.rep(expr) else {
            if self.mentions_rep(rhs) {
                // A plain value computed from positions: a tag, a field, a call's answer.
                self.expr(rhs);
            } else {
                self.expr(rhs);
            }
            return;
        };
        self.depth_of.insert(expr, self.depth);
        let got = match rhs.unspan() {
            Value::Null => {
                self.c.skip.insert(addr(v));
                return;
            }
            Value::Call(callee, args)
                if op_name(self.data, *callee) == "OpRefAlias" && args.len() == 1 =>
            {
                self.c.skip.insert(addr(v));
                return;
            }
            Value::Call(callee, _) if *callee == self.pd => {
                self.expr(rhs);
                Some(Rep::D)
            }
            Value::Call(callee, args)
                if op_name(self.data, *callee) == "OpPlaceRecord"
                    && args.iter().all(|y| !self.mentions_rep(y)) =>
            {
                self.c.skip.insert(addr(v));
                return;
            }
            Value::Call(callee, _) if !op_name(self.data, *callee).starts_with("Op") => {
                self.expr(rhs);
                self.rep_type(self.data.def(*callee).returned())
            }
            Value::Block(bl) if unit_literal(self.data, rhs).is_some() => {
                let _ = bl;
                Some(Rep::T)
            }
            Value::Block(bl) if bl.name == "Object" => {
                if self.object(bl) {
                    Some(Rep::D)
                } else {
                    None
                }
            }
            _ => {
                self.hand_on(rhs);
                self.t_expr(rhs)
            }
        };
        if got != Some(want) {
            let n = self.name(expr);
            self.fail(|| format!("`{n}` is assigned something other than a position"));
        }
    }

    /// The decoder's own record literal: `{ OpDatabase(x); OpSetInt4(x, 0, 0); OpCopyRecord(
    /// <unit literal>, OpGetField(x, vo)); OpSetInt/OpSetBoolean(x, off, scalar)…; x }`.
    fn object(&mut self, bl: &'a Block) -> bool {
        let Some(Value::Var(expr)) = bl.operators.last().map(Value::unspan) else {
            return false;
        };
        let mut scalars = 0;
        let mut value = false;
        for o in &bl.operators[..bl.operators.len() - 1] {
            match o.unspan() {
                Value::Line(_) => {}
                Value::Call(callee, args) => match (op_name(self.data, *callee), args.as_slice()) {
                    ("OpDatabase" | "OpDatabaseNP", [y, _]) if is_var(y, *expr) => {}
                    ("OpSetInt4", [y, z, n])
                        if is_var(y, *expr) && int_of(z) == Some(0) && int_of(n) == Some(0) => {}
                    ("OpCopyRecord", [src, dst, _])
                        if unit_literal(self.data, src).is_some()
                            && matches!(dst.unspan(), Value::Call(h, b) if op_name(self.data, *h) == "OpGetField"
                                && is_var(&b[0], *expr) && int_of(&b[1]) == Some(self.p.vo)) =>
                    {
                        value = true;
                    }
                    ("OpSetInt" | "OpSetBoolean", [y, off, val])
                        if is_var(y, *expr)
                            && int_of(off).is_some_and(|o| self.p.scalar_index(o).is_some()) =>
                    {
                        scalars += 1;
                        self.expr(val);
                    }
                    _ => return false,
                },
                _ => return false,
            }
        }
        value && scalars == self.p.scalars.len()
    }

    fn ret(&mut self, r: &'a Value) {
        if self.in_walk > 0 && self.c.returns.is_none() {
            self.fail(|| {
                "a walk is left by a return of a function that answers no position".into()
            });
            return;
        }
        let Some(want) = self.c.returns else {
            if self.mentions_rep(r) {
                self.fail(|| "a position is returned from a function that answers none".into());
            }
            self.expr(r);
            return;
        };
        self.hand_on(r);
        let got = match r.unspan() {
            Value::Var(expr) if self.rep(*expr).is_some() => {
                self.read(*expr);
                self.rep(*expr)
            }
            Value::Block(_) if unit_literal(self.data, r).is_some() => Some(Rep::T),
            _ => self.t_expr(r),
        };
        if got != Some(want) {
            self.fail(|| "a return of something other than a position".into());
        }
    }

    fn block(&mut self, v: &'a Value, bl: &'a Block) {
        if bl.name == "For block"
            && let Some(walk) = self.walk(bl)
        {
            self.c.walks.insert(addr(v), walk.clone());
            let Some(Value::Loop(lb)) = bl
                .operators
                .iter()
                .map(Value::unspan)
                .find(|o| matches!(o, Value::Loop(_)))
            else {
                return;
            };
            self.depth += 1;
            self.in_walk += 1;
            let saved_depth = self.depth;
            self.depth_of.insert(walk.elem, self.depth);
            for o in &lb.operators[walk.body_from..] {
                self.expr(o);
            }
            self.depth = saved_depth - 1;
            self.in_walk -= 1;
            return;
        }
        if unit_return(self.data, bl).is_some() {
            // `return VNull` written inside the literal's own block.
            if self.c.returns != Some(Rep::T) {
                self.fail(|| {
                    "a unit literal returned from a function that answers no value".into()
                });
            }
            if self.in_walk > 0 && self.c.returns.is_none() {
                self.fail(|| "a walk is left by a return".into());
            }
            return;
        }
        if returns_view(bl) {
            // `return e.value` written into the caller's buffer: the position is the answer.
            self.view_return(v, bl);
            return;
        }
        for o in &bl.operators {
            self.stmt(o);
        }
    }

    /// `{#materialized_view_return OpDatabase(rb); OpCopyRecord(<position>, rb, tp); frees;
    /// return rb }`.
    fn view_return(&mut self, v: &'a Value, bl: &'a Block) {
        let Some(rb) = self.c.retbuf else {
            self.fail(|| "a view return without a buffer".into());
            return;
        };
        let mut src = None;
        for o in &bl.operators {
            match o.unspan() {
                Value::Line(_) => {}
                Value::Return(r) if is_var(r, rb) => {}
                Value::Call(callee, args) => match (op_name(self.data, *callee), args.as_slice()) {
                    ("OpDatabase" | "OpDatabaseNP", [y, _]) if is_var(y, rb) => {}
                    ("OpCopyRecord", [s, d, _]) if is_var(d, rb) => src = Some(s),
                    ("OpFreeText" | "OpFreeRef" | "OpFreeRefIfDistinct", _) => {}
                    _ => {
                        self.fail(|| "a view return that does more than answer".into());
                        return;
                    }
                },
                _ => {
                    self.fail(|| "a view return that does more than answer".into());
                    return;
                }
            }
        }
        match src {
            Some(s)
                if {
                    self.hand_on(s);
                    self.t_expr(s) == Some(Rep::T)
                } =>
            {
                self.c.skip.remove(&addr(v));
            }
            _ => self.fail(|| "a view return of something other than a position".into()),
        }
    }

    fn stmt(&mut self, o: &'a Value) {
        if self.why.is_some() {
            return;
        }
        if self.release_only(o) {
            self.c.skip.insert(addr(o));
            return;
        }
        self.expr(o);
    }

    /// A statement that only releases, clears or mints buffers of positions.
    fn release_only(&self, o: &Value) -> bool {
        if !self.mentions_rep(o) {
            return false;
        }
        !o.any_node(&mut |n| match n {
            Value::Return(_) | Value::Break(_) | Value::Continue(_) => true,
            Value::Set(expr, r) => {
                !(self.c.reps.contains_key(expr)
                    && matches!(r.unspan(), Value::Call(callee, _) if matches!(op_name(self.data, *callee), "OpDatabase" | "OpDatabaseNP" | "OpPlaceRecord")))
            }
            Value::Call(callee, _) => !matches!(
                op_name(self.data, *callee),
                "OpFreeRef"
                    | "OpFreeRefIfDistinct"
                    | "OpClear"
                    | "OpDatabase"
                    | "OpDatabaseNP"
                    | "OpPlaceRecord"
                    | "OpRefIsNull"
                    | "OpConvBoolFromRef"
                    | "OpDistinctStore"
            ),
            _ => false,
        })
    }

    fn if_(&mut self, node: &'a Value, cond: &'a Value, then: &'a Value, otherwise: &'a Value) {
        if self.release_only(node) {
            self.c.skip.insert(addr(node));
            return;
        }
        self.expr(cond);
        if let Some((subject, tag)) = tag_test(self.data, cond) {
            // The arm's value may not change inside the arm.
            let vars: Vec<u16> = vars_of(subject);
            if vars
                .iter()
                .any(|var| then.any_node(&mut |n| matches!(n, Value::Set(set, _) if set == var)))
            {
                self.fail(|| "a matched value is assigned inside its arm".into());
            }
            self.arms.push((subject, tag));
            self.expr(then);
            self.arms.pop();
        } else {
            self.expr(then);
        }
        self.expr(otherwise);
    }

    /// `for e in v { body }` over a position vector, in either form the parser writes:
    /// `{#For block _vector = v; e#index = -1; loop { e = {#iter next …}; exits; body… } }`,
    /// or with the length taken once before the loop.
    fn walk(&mut self, bl: &'a Block) -> Option<Walk> {
        let ops: Vec<&Value> = bl
            .operators
            .iter()
            .map(Value::unspan)
            .filter(|op| !matches!(op, Value::Line(_)))
            .collect();
        let (vv, src, idx, init, lb, len) = match ops.as_slice() {
            [Value::Set(vv, src), Value::Set(idx, init), Value::Loop(lb)] => {
                (*vv, src, *idx, init, lb, None)
            }
            [
                Value::Set(vv, src),
                Value::Set(len, len_rhs),
                Value::Set(idx, init),
                Value::Loop(lb),
            ] if matches!(len_rhs.unspan(), Value::Call(op, args)
                if op_name(self.data, *op) == "OpLengthVector"
                    && args.len() == 1 && is_var(&args[0], *vv)) =>
            {
                (*vv, src, *idx, init, lb, Some(*len))
            }
            _ => return None,
        };
        let Some(Rep::V(vtp)) = self.rep(vv) else {
            return None;
        };
        if int_of(init) != Some(-1) {
            return None;
        }
        // The owner: `src` is a vector field read inside the arm that proved it.
        self.depth_of.insert(vv, self.depth);
        self.hand_on(src);
        let owner = match src.unspan() {
            Value::Var(local) if self.rep(*local) == Some(Rep::V(vtp)) => {
                self.read(*local);
                true
            }
            _ => self.t_expr(src) == Some(Rep::V(vtp)),
        };
        if !owner {
            return None;
        }
        let field = self.vector_field(src, vtp)?;
        let loop_ops: Vec<(usize, &Value)> = lb
            .operators
            .iter()
            .enumerate()
            .map(|(i, op)| (i, op.unspan()))
            .filter(|(_, op)| !matches!(op, Value::Line(_)))
            .collect();
        let head = WalkHead {
            vv,
            idx,
            field,
            vtp,
        };
        match len {
            Some(len) => self.counted_walk(&loop_ops, &head, len),
            None => self.next_walk(&loop_ops, lb, &head),
        }
    }

    /// `loop { e = {#iter next idx += 1; OpGetVectorNullable(_vector, size, idx)};
    /// if len(_vector) <= idx break; if idx < 0 break; body… }`.
    fn next_walk(
        &mut self,
        loop_ops: &[(usize, &'a Value)],
        lb: &'a Block,
        head: &WalkHead,
    ) -> Option<Walk> {
        let [
            (_, Value::Set(elem, next)),
            (_, exit1),
            (_, exit2),
            rest @ ..,
        ] = loop_ops
        else {
            return None;
        };
        let Value::Block(next_block) = next.unspan() else {
            return None;
        };
        let next_ops: Vec<&Value> = next_block
            .operators
            .iter()
            .map(Value::unspan)
            .filter(|op| !matches!(op, Value::Line(_)))
            .collect();
        let [Value::Set(stepped, step), last] = next_ops.as_slice() else {
            return None;
        };
        if *stepped != head.idx
            || !self.steps(step, head.idx)
            || !self.reads_element(last, head.vv, head.idx)
            || !Self::exits(exit1, head)
            || !Self::exits(exit2, head)
        {
            return None;
        }
        let body_from = rest.first().map_or(lb.operators.len(), |(i, _)| *i);
        self.walk_tail(*elem, rest, &[head.idx, head.vv], head, body_from)
    }

    /// `loop { at = {#Iter range idx += 1; if len <= idx break; idx }; e =
    /// OpGetVectorNullable(_vector, size, at); body… }`.
    fn counted_walk(
        &mut self,
        loop_ops: &[(usize, &'a Value)],
        head: &WalkHead,
        len: u16,
    ) -> Option<Walk> {
        let [
            (_, Value::Set(at, iter)),
            (elem_at, Value::Set(elem, get)),
            rest @ ..,
        ] = loop_ops
        else {
            return None;
        };
        let Value::Block(iter_block) = iter.unspan() else {
            return None;
        };
        let range_ops: Vec<&Value> = iter_block
            .operators
            .iter()
            .map(Value::unspan)
            .filter(|op| !matches!(op, Value::Line(_)))
            .collect();
        let [
            Value::Set(stepped, step),
            Value::If(cond, _, _),
            Value::Var(last),
        ] = range_ops.as_slice()
        else {
            return None;
        };
        if *stepped != head.idx
            || *last != head.idx
            || !self.steps(step, head.idx)
            || !cond.any_node(&mut |n| matches!(n, Value::Var(var) if *var == len))
            || !self.reads_element(get, head.vv, *at)
        {
            return None;
        }
        let body_from = rest.first().map_or(elem_at + 1, |(i, _)| *i);
        self.walk_tail(*elem, rest, &[head.idx, head.vv, len, *at], head, body_from)
    }

    /// `idx = OpAddInt(idx, 1)`.
    fn steps(&self, step: &Value, idx: u16) -> bool {
        matches!(step.unspan(), Value::Call(op, args)
            if op_name(self.data, *op).starts_with("OpAddInt")
                && is_var(&args[0], idx) && int_of(&args[1]) == Some(1))
    }

    /// `OpGetVectorNullable(vv, size, at)`, through an enum element's field read.
    fn reads_element(&self, get: &Value, vv: u16, at: u16) -> bool {
        matches!(element_read(self.data, get).unspan(), Value::Call(op, args)
            if op_name(self.data, *op) == "OpGetVectorNullable"
                && is_var(&args[0], vv) && is_var(&args[2], at))
    }

    /// `if <a test of the index> { break }` — one of the two exits of the parser's walk.
    fn exits(node: &Value, head: &WalkHead) -> bool {
        let Value::If(cond, then, otherwise) = node.unspan() else {
            return false;
        };
        let breaks = match then.unspan() {
            Value::Break(0) => true,
            Value::Block(tb) => matches!(
                tb.operators
                    .iter()
                    .map(Value::unspan)
                    .filter(|op| !matches!(op, Value::Line(_)))
                    .collect::<Vec<_>>()
                    .as_slice(),
                [Value::Break(0)]
            ),
            _ => false,
        };
        breaks
            && matches!(otherwise.unspan(), Value::Null)
            && cond.any_node(&mut |n| matches!(n, Value::Var(var) if *var == head.idx))
            && !cond.any_node(
                &mut |n| matches!(n, Value::Var(var) if *var != head.idx && *var != head.vv),
            )
    }

    /// The element's type and the body's independence of the loop's own variables.
    fn walk_tail(
        &mut self,
        elem: u16,
        rest: &[(usize, &'a Value)],
        own: &[u16],
        head: &WalkHead,
        body_from: usize,
    ) -> Option<Walk> {
        let (etp, whole) = *self.p.elem.get(&head.vtp)?;
        let want = if whole { Rep::T } else { Rep::E(etp) };
        if self.rep(elem) != Some(want) {
            return None;
        }
        // The body reads none of the loop's own variables.
        for (_, op) in rest {
            if op.any_node(
                &mut |n| matches!(n, Value::Var(var) | Value::Set(var, _) if own.contains(var)),
            ) {
                return None;
            }
        }
        Some(Walk {
            elem,
            vec: head.vv,
            field: head.field,
            body_from,
        })
    }

    /// The `(offset, vector type)` a vector position names: its `OpGetField`, through
    /// copies of position locals.
    fn vector_field(&self, src: &Value, vtp: u16) -> Option<(i64, u16)> {
        match src.unspan() {
            Value::Call(op, args) if op_name(self.data, *op) == "OpGetField" => {
                Some((int_of(&args[1])?, vtp))
            }
            Value::Var(local) => {
                // The local's one assignment from a field read.
                let mut found = None;
                self.data.def(self.d).code().any_node(&mut |n| {
                    if let Value::Set(set, rhs) = n
                        && set == local
                        && let Value::Call(op, args) = rhs.unspan()
                        && op_name(self.data, *op) == "OpGetField"
                    {
                        found = int_of(&args[1]);
                    }
                    false
                });
                Some((found?, vtp))
            }
            _ => None,
        }
    }
}

/// The parts of a walk the loop's head names.
struct WalkHead {
    /// The vector variable the loop reads.
    vv: u16,
    /// The index variable.
    idx: u16,
    field: (i64, u16),
    vtp: u16,
}

/// An element read: `OpGetVectorNullable(v, size, i)`, seen through the `OpGetField(…, 0, T)`
/// a vector of enum values wraps it in.
fn element_read<'v>(data: &Data, node: &'v Value) -> &'v Value {
    match node.unspan() {
        Value::Call(op, args)
            if op_name(data, *op) == "OpGetField"
                && args.len() == 3
                && int_of(&args[1]) == Some(0) =>
        {
            &args[0]
        }
        _ => node,
    }
}

fn is_var(node: &Value, var: u16) -> bool {
    matches!(node.unspan(), Value::Var(v) if *v == var)
}

fn vars_of(node: &Value) -> Vec<u16> {
    let mut out = Vec::new();
    node.any_node(&mut |n| {
        if let Value::Var(var) = n {
            out.push(*var);
        }
        false
    });
    out
}

/// `OpEqInt(OpConvIntFromEnum(OpGetEnum(x, 0)), tag)`: answers `(x, tag)`.
pub(crate) fn tag_test<'a>(data: &Data, cond: &'a Value) -> Option<(&'a Value, u8)> {
    let Value::Call(eq, eq_args) = cond.unspan() else {
        return None;
    };
    if op_name(data, *eq) != "OpEqInt" || eq_args.len() != 2 {
        return None;
    }
    let Value::Call(to_int, converted) = eq_args[0].unspan() else {
        return None;
    };
    if op_name(data, *to_int) != "OpConvIntFromEnum" {
        return None;
    }
    let Value::Call(get, get_args) = converted.first()?.unspan() else {
        return None;
    };
    if op_name(data, *get) != "OpGetEnum" || int_of(get_args.get(1)?) != Some(0) {
        return None;
    }
    let tag = u8::try_from(int_of(&eq_args[1])?).ok()?;
    Some((&get_args[0], tag))
}

/// The view return the scope pass writes for `return e.value` into the caller's buffer.
fn returns_view(bl: &Block) -> bool {
    bl.name == "materialized_view_return"
}

fn elem_fields(stores: &Stores, etp: u16) -> Option<&Vec<crate::database::Field>> {
    match stores.types.get(etp as usize).map(|t| &t.parts) {
        Some(Parts::Struct(fields)) => Some(fields),
        _ => None,
    }
}

/// The fields `(offset, content type)` of the variant of enum `t_tp` with tag `tag`.
pub(crate) fn variant_fields(stores: &Stores, t_tp: u16, tag: u8) -> Option<Vec<(i64, u16)>> {
    let Some(Parts::Enum(variants)) = stores.types.get(t_tp as usize).map(|t| &t.parts) else {
        return None;
    };
    for (vtp, _) in variants {
        if let Some(Parts::EnumValue(variant, fields)) =
            stores.types.get(*vtp as usize).map(|t| &t.parts)
            && *variant == tag
        {
            return Some(
                fields
                    .iter()
                    .map(|field| (i64::from(field.position), field.content))
                    .collect(),
            );
        }
    }
    None
}

/// What a loft type holds as a position, if anything.
pub(crate) fn rep_of(data: &Data, prod: &Producer, tp: &Type) -> Option<Rep> {
    if let Type::Vector(elem, _) = tp.base() {
        let etp = record_tp(data, elem)?;
        return prod
            .elem
            .iter()
            .find(|(_, (found, _))| *found == etp)
            .map(|(vtp, _)| Rep::V(*vtp));
    }
    let rec = record_tp(data, tp)?;
    if rec == prod.d_tp {
        Some(Rep::D)
    } else if rec == prod.t_tp {
        Some(Rep::T)
    } else if prod
        .elem
        .values()
        .any(|(found, whole)| *found == rec && !whole)
    {
        Some(Rep::E(rec))
    } else {
        None
    }
}

/// Scan consumer `d` of producer `pd`.
fn consumer(
    data: &Data,
    stores: &Stores,
    dv: &DvPlan,
    plan: &PsPlan,
    pd: u32,
    d: u32,
    root: bool,
) -> Result<(Consumer, Vec<u32>), String> {
    let p = &plan.producers[&pd];
    let def = data.def(d);
    let vars = def.variables();
    let mut c = Consumer {
        producer: pd,
        ..Consumer::default()
    };
    let frames = dv.bparams.get(&d).cloned().unwrap_or_default();
    if root {
        c.frame = dv.roots.get(&d).copied();
    } else {
        c.frame = frames.iter().copied().min();
        if frames.len() > 1 {
            return Err("two frame parameters".into());
        }
    }
    c.returns = rep_of(data, p, def.returned());
    if root && c.returns.is_some() {
        return Err("the root answers a decoded value".into());
    }
    if c.returns.is_some() {
        let rb = super::hoist::ret_buffer_attr(def).ok_or("answers a position without a buffer")?;
        c.retbuf = Some(vars.var(&def.attributes()[rb].name));
    }
    for v in 0..vars.count() {
        if let Some(r) = rep_of(data, p, vars.tp(v)) {
            c.reps.insert(v, r);
        }
    }
    if c.reps.is_empty() {
        return Err("holds no decoded value".into());
    }
    if std::env::var("LOFT_TRACE_DECODE_STRUCTURE").is_ok_and(|v| v == "2") {
        let mut w = Vec::new();
        let mut vs = vars.clone();
        let _ = data.show_code(&mut w, &mut vs, def.code(), 0, true);
        eprintln!(
            "[decode-structure] IR of {}:\n{}",
            def.name(),
            String::from_utf8_lossy(&w)
        );
    }
    let mut s = CScan {
        data,
        stores,
        plan,
        p,
        pd,
        d,
        c,
        depth_of: HashMap::default(),
        depth: 0,
        arms: Vec::new(),
        callees: Vec::new(),
        why: None,
        in_walk: 0,
        uses: HashMap::default(),
    };
    for args in vars.arguments() {
        s.depth_of.insert(args, 0);
    }
    if let Value::Block(bl) = def.code() {
        for o in &bl.operators {
            s.stmt(o);
        }
    } else {
        return Err("no body".into());
    }
    if let Some(why) = s.why {
        return Err(why);
    }
    s.c.callees.clone_from(&s.callees);
    Ok((s.c, s.callees))
}

/// Admit each decode-view root whose chain holds every decoded value as a position.
pub fn roots(data: &Data, stores: &Stores, dv: &DvPlan, plan: &mut PsPlan) {
    let mut roots: Vec<u32> = dv.roots.keys().copied().collect();
    roots.sort_unstable();
    let producers: Vec<u32> = plan.producers.keys().copied().collect();
    for r in roots {
        // The producer this root's chain reaches: the one whose record a call answers.
        let mut admitted: HashMap<u32, Consumer> = HashMap::default();
        let mut result = Err("reaches no producer".to_string());
        for &pd in &producers {
            let mut queue = vec![(r, true)];
            let mut seen: HashSet<u32> = HashSet::default();
            let mut local: HashMap<u32, Consumer> = HashMap::default();
            let mut err = None;
            while let Some((callee, is_root)) = queue.pop() {
                if !seen.insert(callee) {
                    continue;
                }
                if let Some(known) = plan.consumers.get(&callee) {
                    for callee in &known.callees {
                        queue.push((*callee, false));
                    }
                    local.insert(callee, known.clone());
                    continue;
                }
                match consumer(data, stores, dv, plan, pd, callee, is_root) {
                    Ok((c, callees)) => {
                        for callee in callees {
                            if callee == r {
                                err = Some("the root reaches itself".to_string());
                            }
                            queue.push((callee, false));
                        }
                        local.insert(callee, c);
                    }
                    Err(why) => {
                        err = Some(format!("{}: {why}", data.def(callee).name()));
                        break;
                    }
                }
            }
            // A consumer chain that never reaches the producer holds no decoded value.
            let reaches = local
                .values()
                .any(|c| c.reps.values().any(|r| *r == Rep::D));
            match err {
                None if reaches => {
                    admitted = local;
                    result = Ok(pd);
                    break;
                }
                None => {}
                Some(why) => result = Err(why),
            }
        }
        match result {
            Ok(pd) => {
                trace(|| {
                    let mut names: Vec<&str> = admitted
                        .keys()
                        .map(|callee| data.def(*callee).name())
                        .collect();
                    names.sort_unstable();
                    format!(
                        "{}: a root over {}, consumers [{}]",
                        data.def(r).name(),
                        data.def(pd).name(),
                        names.join(", ")
                    )
                });
                for c in admitted.values() {
                    for w in c.walks.values() {
                        let n = plan.walks.len();
                        plan.walks.entry((pd, w.field.0, w.field.1)).or_insert(n);
                    }
                }
                plan.consumers.extend(admitted);
                plan.roots.insert(r);
            }
            Err(why) => trace(|| format!("{}: declines: {why}", data.def(r).name())),
        }
    }
}
