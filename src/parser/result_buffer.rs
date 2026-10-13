// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-WorkBuffer`'s result clause — a call's vector result that is only READ where it
//! stands is filled into the CALLER's buffer.
//!
//! `fn width(…) { runs = slice(…); for r in runs { … } }` mints the store `slice` fills on
//! every call of `width` and frees it at the exit.  The function never owns that value in
//! any way the source can see: it reads the length, the elements and their fields, and the
//! result is gone when it returns.  So the buffer is a hidden work-buffer parameter, as a
//! non-escaping vector local is (`work_buffer.rs`): the caller's frame keeps the store
//! across the function's activations, the transitive clause carries it on to the frame
//! that loops, and a callee that refills a live buffer (`@FR-R-RefillBuffer`,
//! `@FR-R-RefillText`'s collection clause) finds the previous call's elements in it.
//!
//! The read walk IS the escape proof: a view of the buffer that is only read cannot be
//! copied, moved, adopted or returned, so nothing of the store outlives the activation.
//! Every walk's fallback is a DECLINE, which keeps the mint the program already pays.
//!
//! `LOFT_NO_RESULT_BUFFER=1` keeps every such buffer in its frame; `LOFT_TRACE_WORK_BUFFER=1`
//! names each ref promoted and each declined with the reason (`result=`).
use std::collections::HashSet;

use super::Parser;
use super::work_buffer::{
    CallGraph, is_scalar_accessor, is_var, reaches, set_counts, var_mentions,
};
use crate::data::{Data, DefType, Deps, Type, Value, v_if, v_set};
use crate::variables::Function;

impl Parser {
    /// The result clause for one definition: the number of its result refs promoted.
    ///
    /// A work-ref this frame mints for a callee's hidden VECTOR return buffer, whose result
    /// is only read where it stands (`admit_result_ref`), becomes a hidden work-buffer
    /// parameter: the caller's frame keeps the store across this function's activations, so
    /// a callee that refills a live buffer (`@FR-R-RefillBuffer`, `@FR-R-RefillText`'s
    /// collection clause) finds the previous call's elements where a store minted per
    /// activation handed it an empty one.  Handed the null sentinel the function takes the
    /// rebound-parameter road at entry, as `promote_one` writes it: an entry witness, a store
    /// of its own, released at exit against the witness.
    ///
    /// Only for a function that `runs_again`: the caller's buffer pays where the function
    /// is entered a second time while that buffer lives.  A function called once per
    /// activation of everything above it would hand its result's store to a frame that
    /// never uses it again, and hold it there past the scope the source ends it in.
    pub(super) fn promote_result_refs_in(
        &mut self,
        d: u32,
        calls: &CallGraph,
        again: bool,
    ) -> usize {
        let refs = result_refs(self.data.def(d));
        if refs.is_empty() {
            return 0;
        }
        let saved_ctx = self.context;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d as usize].variables,
        );
        self.context = d;
        self.vars.sync_work_counters();
        let mut code = std::mem::replace(&mut self.data.definitions[d as usize].code, Value::Null);
        let trace = crate::keys::trace_work_buffer();
        let mut promoted = 0;
        for r in refs {
            let verdict = if again {
                self.admit_result_ref(&code, r, calls)
            } else {
                Err("no call of this function stands in a loop")
            };
            match verdict {
                Ok(elem) => {
                    self.promote_result_ref(&mut code, r, &elem);
                    promoted += 1;
                    crate::rewrite_census::fired("R-WorkBuffer/result", 1);
                    if trace {
                        eprintln!(
                            "[work-buffer] fn={} result={} PROMOTED to a caller buffer",
                            self.data.def(d).name(),
                            self.vars.name(r)
                        );
                    }
                }
                Err(why) => {
                    if trace {
                        eprintln!(
                            "[work-buffer] fn={} result={} keeps its mint: {why}",
                            self.data.def(d).name(),
                            self.vars.name(r)
                        );
                    }
                }
            }
        }
        self.data.definitions[d as usize].code = code;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d as usize].variables,
        );
        self.context = saved_ctx;
        promoted
    }

    /// The result clause's admission of work-ref `r` of the current definition; answers its
    /// element type.  `r` is named once, as the hidden return buffer of ONE call of a
    /// callee that cannot reach back here; that call is the whole value of the one bind of
    /// a plain local, a view of `r` and of nothing else; and that local, every view bound
    /// from it and every element read from it is only READ where it stands (`ReadWalk`).
    fn admit_result_ref(
        &self,
        code: &Value,
        r: u16,
        calls: &CallGraph,
    ) -> Result<Type, &'static str> {
        let tp = self.vars.tp(r);
        if tp.peel_optional().1 {
            return Err("a nullable buffer");
        }
        let Type::Vector(elem, deps) = tp.base() else {
            return Err("not a vector");
        };
        if !deps.is_empty() {
            return Err("a view of another store");
        }
        if self.vars.arguments().iter().any(|&a| a > r) {
            return Err("numbered before an argument");
        }
        let Value::Block(bl) = code.unspan() else {
            return Err("no body");
        };
        let top_init = bl
            .operators
            .iter()
            .filter(|op| matches!(op.unspan(), Value::Set(x, rhs) if *x == r && matches!(rhs.unspan(), Value::Null)))
            .count();
        if top_init != 1 || set_counts(code).get(&r).copied().unwrap_or(0) != 1 {
            return Err("assigned other than by its entry null-init");
        }
        if var_mentions(code, r) != 1 {
            return Err("handed to more than one call");
        }
        let x = self.result_bind(code, r, calls)?;
        let mut walk = ReadWalk {
            data: &self.data,
            function: &self.vars,
            r,
            vecs: HashSet::from([x]),
            elems: HashSet::new(),
            grew: false,
        };
        // A view found on one round is judged on the next, and so is the value it stands in
        // (`r = { e = v[i]; if e { e } else { … } }` binds `e` inside the value that yields
        // it): a round that found one is run again whatever it answered.  The sets only grow.
        loop {
            walk.grew = false;
            let verdict = walk.node(code, Ctx::Other);
            if !walk.grew {
                verdict?;
                break;
            }
        }
        Ok((**elem).clone())
    }

    /// The one bind of buffer `r`: `x = callee(…, r, …)`, the call the bind's whole value,
    /// `r` at a hidden vector return buffer of a loft callee that cannot reach back here,
    /// and `x` a plain local bound once, a view of `r` and of nothing else.  Answers `x`.
    fn result_bind(&self, code: &Value, r: u16, calls: &CallGraph) -> Result<u16, &'static str> {
        let Some(Value::Set(x, rhs)) = find_bind(code, r) else {
            return Err("its call is not the value of a bind");
        };
        let Value::Call(callee, args) = rhs.unspan() else {
            return Err("its call is not the value of a bind");
        };
        let (x, callee) = (*x, *callee);
        let at = args.iter().position(|a| is_var(a, r)).unwrap_or(usize::MAX);
        let def = self.data.def(callee);
        if def.def_type() != DefType::Function
            || !def.rust().is_empty()
            || !matches!(def.code().unspan(), Value::Block(_))
        {
            return Err("its callee has no loft body");
        }
        if !def.attributes().get(at).is_some_and(|a| {
            a.hidden && !a.work_buffer && matches!(a.typedef.base(), Type::Vector(_, _))
        }) {
            return Err("handed to other than a vector return buffer");
        }
        if reaches(calls, callee, self.context) {
            return Err("its callee is on a cycle with this function");
        }
        // `@FR-R-LazySplit` and `@FR-R-SplitTable` read a constant split without building
        // its vector at all, and leave the buffer unminted: a caller's buffer for it
        // would be a store nothing fills.
        if crate::generation::hoist::split_call(rhs, &self.data, &self.vars).is_ok() {
            return Err("a constant split, read without its vector");
        }
        if self.vars.is_argument(x) || self.vars.tp(x).peel_optional().1 {
            return Err("its result is bound to a parameter or a nullable local");
        }
        if !matches!(self.vars.tp(x).base(), Type::Vector(_, _)) || self.vars.tp(x).depend() != [r]
        {
            return Err("its result is not a view of this buffer alone");
        }
        let binds = {
            let mut n = 0;
            code.walk(&mut |v| {
                if let Value::Set(y, rhs) = v
                    && *y == x
                    && !matches!(rhs.unspan(), Value::Null)
                {
                    n += 1;
                }
            });
            n
        };
        if binds != 1 {
            return Err("its result local is assigned again");
        }
        // `@FR-R-ElemFirst`'s loop source: a call a `for` iterates is the native emitter's
        // to place (`scopes::buffers::iterates_a_call_into`), keyed on the buffer being a local.
        if code.any_node(&mut |n| {
            matches!(n, Value::Block(b) if b.name == "For block"
                && matches!(b.operators.first().map(Value::unspan),
                    Some(Value::Set(_, call)) if matches!(call.unspan(),
                        Value::Call(_, args) if args.iter().any(|a| is_var(a, r)))))
        }) {
            return Err("its call is iterated by a loop");
        }
        Ok(x)
    }

    /// The promotion of one admitted result ref: the hidden attribute, the argument, and at
    /// entry the witness's stash and the null road's mint.
    fn promote_result_ref(&mut self, code: &mut Value, buf: u16, elem: &Type) {
        let def_nr = self.context;
        let name = self.vars.name(buf).to_string();
        let buf_tp = Type::Vector(Box::new(elem.clone()), Deps::none());
        let attr_nr = self
            .data
            .add_attribute(&mut self.lexer, def_nr, &name, buf_tp);
        let attr = &mut self.data.definitions[def_nr as usize].attributes[attr_nr];
        attr.hidden = true;
        attr.work_buffer = true;
        self.vars.become_argument(buf);
        self.vars.retire_work_buffer_ref(buf);
        let witness = self.ensure_rebind_witness(buf);
        // The rebound-parameter mint (`vector_db` on a parameter with a witness): the
        // guarded free of what the parameter held, a fresh backing, the parameter bound as
        // its view.  The backing is new to this body, so its null-init is written here.
        let mint = self.vector_db(elem, buf);
        let backing = self.vars.tp(buf).depend().last().copied();
        let is_null = self.cl("OpRefIsNull", &[Value::Var(buf)]);
        let stash = self.cl("OpPutRef", &[Value::Var(witness), Value::Var(buf)]);
        let null_init = |op: &Value, of: Option<u16>| {
            matches!(op.unspan(), Value::Set(x, rhs)
                if of.is_none_or(|v| v == *x) && matches!(rhs.unspan(), Value::Null))
        };
        if let Value::Block(bl) = code.unspan_mut() {
            bl.operators.retain(|op| !null_init(op, Some(buf)));
            if let Some(db) = backing {
                bl.operators.insert(0, v_set(db, Value::Null));
            }
            bl.operators.insert(0, v_set(witness, Value::Null));
            // The stash at entry, after the null-inits; the null road's mint in front of
            // the top-level statement that makes the call, so an exit before it mints
            // nothing and its free of the buffer it was handed declines on the witness.
            let at = bl
                .operators
                .iter()
                .position(|op| !null_init(op, None))
                .unwrap_or(bl.operators.len());
            let call_at = bl
                .operators
                .iter()
                .position(|op| var_mentions(op, buf) > 0)
                .unwrap_or(at);
            bl.operators
                .insert(call_at, v_if(is_null, Value::Insert(mint), Value::Null));
            bl.operators.insert(at, stash);
        }
    }
}

/// The `Set(x, callee(…, r, …))` that hands buffer `r` to a call as the bind's whole value.
fn find_bind(code: &Value, r: u16) -> Option<&Value> {
    let mut bind = None;
    code.walk(&mut |x| {
        if let Value::Set(_, rhs) = x
            && let Value::Call(_, args) = rhs.unspan()
            && args.iter().any(|a| is_var(a, r))
        {
            bind = Some(x);
        }
    });
    bind
}

/// The candidates of one definition: each `__ref_N` it mints for a call's hidden VECTOR
/// buffer and owns outright.  A record buffer, a view, a witness and a buffer this frame
/// already hands down for a callee's work buffer are other rules' subjects.
pub(super) fn result_refs(def: &crate::data::Definition) -> Vec<u16> {
    let vars = &def.variables;
    (0..vars.count())
        .filter(|&r| {
            vars.is_caller_hidden_buf(r)
                && !vars.is_work_buffer_ref(r)
                && !vars.is_argument(r)
                && !vars.is_inline_ref(r)
                && !vars.is_skip_free(r)
                && vars.name(r).starts_with("__ref_")
                && !vars.tp(r).peel_optional().1
                && matches!(vars.tp(r).base(), Type::Vector(_, deps) if deps.is_empty())
        })
        .collect()
}

/// Per definition: can it be entered more than once during one activation of a frame above
/// it — a direct call of it stands in a loop, or in a function that itself runs again or
/// calls itself?  `false` for everything this cannot see repeat (a function reached only
/// through a function value, a `par` worker, an entry point): that keeps a buffer where the
/// source puts it, which costs the mint it already pays.
pub(super) fn runs_again(data: &Data, calls: &CallGraph) -> Vec<bool> {
    fn looped(v: &Value, in_loop: bool, out: &mut [bool]) {
        if in_loop && let Value::Call(c, _) = v {
            out[*c as usize] = true;
        }
        let inner = in_loop || matches!(v, Value::Loop(_));
        v.for_each_child(&mut |c| looped(c, inner, out));
    }
    let mut again = vec![false; data.definitions.len()];
    for d in 0..data.definitions.len() as u32 {
        looped(data.def(d).code(), false, &mut again);
    }
    loop {
        let mut grew = false;
        for d in 0..calls.len() {
            let callees = calls.of(d as u32);
            if again[d] || callees.contains(&(d as u32)) {
                for &c in callees {
                    if !again[c as usize] {
                        again[c as usize] = true;
                        grew = true;
                    }
                }
            }
        }
        if !grew {
            return again;
        }
    }
}

/// Where a node stands for `ReadWalk`: its value discarded, read as a vector or as an
/// element by the operator above it, bound to a local, or anything else.
#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    Other,
    Discard,
    ReadVec,
    ReadElem,
    Flow,
}

/// `@FR-R-WorkBuffer`'s result clause — is every view of buffer `r` only READ where it
/// stands?  `vecs` are the locals that view the buffer's vector (the result local, a loop's
/// alias of it), `elems` the locals bound to an element of it.  Any mention this does not
/// name is a decline, which keeps the buffer a local of the frame.
struct ReadWalk<'a> {
    data: &'a Data,
    function: &'a Function,
    r: u16,
    vecs: HashSet<u16>,
    elems: HashSet<u16>,
    grew: bool,
}

/// An operator that reads its first operand's vector and changes nothing.
fn vector_read_at_0(name: &str) -> bool {
    matches!(
        name,
        "OpLengthVector" | "OpVectorIsNull" | "OpGetVector" | "OpGetVectorNullable"
    )
}

/// An operator that reads through its first operand's record or element place.
fn element_read_at_0(name: &str) -> bool {
    (is_scalar_accessor(name) && name.starts_with("OpGet"))
        || matches!(name, "OpGetText" | "OpConvBoolFromRef")
}

impl ReadWalk<'_> {
    fn tracked(&self, x: u16) -> bool {
        self.vecs.contains(&x) || self.elems.contains(&x)
    }

    /// A one-op stdlib wrapper (`len(v)`) reads as its op (`hoist::one_op_wrapper`).
    fn op_name(&self, d: u32) -> &str {
        use crate::generation::hoist::WrapperOperand;
        if let Some((op, operands)) = crate::generation::hoist::one_op_wrapper(self.data, d)
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

    /// Can `v` evaluate to a view of the buffer: `Some(true)` its vector, `Some(false)` an
    /// element of it.  `None` for every shape not named: such a value is judged where its
    /// own nodes stand, and a tracked name among them outside a read declines.
    fn yields(&self, v: &Value) -> Option<bool> {
        match v.unspan() {
            Value::Var(x) if self.vecs.contains(x) => Some(true),
            Value::Var(x) if self.elems.contains(x) => Some(false),
            Value::Call(d, args)
                if matches!(self.op_name(*d), "OpGetVector" | "OpGetVectorNullable")
                    && args.first().is_some_and(
                        |a| matches!(a.unspan(), Value::Var(x) if self.vecs.contains(x)),
                    ) =>
            {
                Some(false)
            }
            Value::Block(bl) => bl.operators.last().and_then(|l| self.yields(l)),
            Value::Insert(xs) => xs.last().and_then(|l| self.yields(l)),
            Value::If(_, a, b) => self.yields(a).or_else(|| self.yields(b)),
            _ => None,
        }
    }

    fn list(&mut self, ops: &[Value], tail: Ctx) -> Result<(), &'static str> {
        for (i, op) in ops.iter().enumerate() {
            let ctx = if i + 1 == ops.len() {
                tail
            } else {
                Ctx::Discard
            };
            self.node(op, ctx)?;
        }
        Ok(())
    }

    /// A call: a reading operator's first operand is read as a vector or as an element,
    /// every other operand of any call is judged as any other mention.
    fn call(&mut self, node: &Value, d: u32, args: &[Value], ctx: Ctx) -> Result<(), &'static str> {
        let name = self.op_name(d);
        let first = if vector_read_at_0(name) {
            if self.yields(node) == Some(false)
                && !matches!(ctx, Ctx::ReadElem | Ctx::Discard | Ctx::Flow)
            {
                return Err("an element of its result is mentioned outside a read");
            }
            Ctx::ReadVec
        } else if element_read_at_0(name) {
            Ctx::ReadElem
        } else {
            Ctx::Other
        };
        for (i, a) in args.iter().enumerate() {
            self.node(a, if i == 0 { first } else { Ctx::Other })?;
        }
        Ok(())
    }

    /// A bind: the buffer's own call, a null-init, a local bound to a view of the buffer
    /// (tracked from here on), or any other value.
    fn bind(&mut self, x: u16, rhs: &Value) -> Result<(), &'static str> {
        if matches!(rhs.unspan(), Value::Null) {
            return Ok(());
        }
        if let Value::Call(_, args) = rhs.unspan()
            && args.iter().any(|a| is_var(a, self.r))
        {
            let r = self.r;
            for a in args.iter().filter(|a| !is_var(a, r)) {
                self.node(a, Ctx::Other)?;
            }
            return Ok(());
        }
        let Some(is_vec) = self.yields(rhs) else {
            return self.node(rhs, Ctx::Other);
        };
        if self.function.is_argument(x) {
            return Err("its result is copied into a parameter");
        }
        // A local whose type borrows is a VIEW of what it is bound from; one that
        // borrows nothing took a copy, which the move rewrites may turn into an
        // adoption of the store — and those ask for a local's store.
        let borrows = self
            .function
            .tp(x)
            .depend()
            .iter()
            .any(|&dep| dep == self.r || self.tracked(dep));
        if !borrows {
            return Err("its result is copied whole into a local");
        }
        let set = if is_vec {
            &mut self.vecs
        } else {
            &mut self.elems
        };
        if set.insert(x) {
            self.grew = true;
        }
        self.node(rhs, Ctx::Flow)
    }

    fn node(&mut self, node: &Value, ctx: Ctx) -> Result<(), &'static str> {
        match node.unspan() {
            Value::Var(x) if self.vecs.contains(x) => {
                if matches!(ctx, Ctx::ReadVec | Ctx::Discard | Ctx::Flow) {
                    Ok(())
                } else {
                    Err("its result is mentioned outside a read")
                }
            }
            Value::Var(x) if self.elems.contains(x) => {
                if matches!(ctx, Ctx::ReadElem | Ctx::Discard | Ctx::Flow) {
                    Ok(())
                } else {
                    Err("an element of its result is mentioned outside a read")
                }
            }
            Value::Call(d, args) => self.call(node, *d, args, ctx),
            Value::Set(x, rhs) => self.bind(*x, rhs),
            Value::Iter(x, a, b, c) => {
                if self.tracked(*x) {
                    return Err("its result is bound by a loop");
                }
                self.node(a, Ctx::Other)?;
                self.node(b, Ctx::Other)?;
                self.node(c, Ctx::Other)
            }
            Value::TupleGet(base, _) => {
                if self.tracked(*base) {
                    Err("its result is in a tuple")
                } else {
                    Ok(())
                }
            }
            Value::TuplePut(base, _, inner) => {
                if self.tracked(*base) {
                    return Err("its result is in a tuple");
                }
                self.node(inner, Ctx::Other)
            }
            Value::CallRef(f, xs) => {
                if self.tracked(*f) {
                    return Err("its result is called as a function value");
                }
                for x in xs {
                    self.node(x, Ctx::Other)?;
                }
                Ok(())
            }
            Value::FnRef(_, clos, _) => {
                if self.tracked(*clos) {
                    Err("its result is captured by a closure")
                } else {
                    Ok(())
                }
            }
            Value::FnRefDnr(x) => {
                if self.tracked(*x) {
                    Err("its result is read as a function reference")
                } else {
                    Ok(())
                }
            }
            Value::Block(bl) => self.list(&bl.operators, ctx),
            Value::Loop(bl) => self.list(&bl.operators, Ctx::Discard),
            Value::Insert(xs) => self.list(xs, ctx),
            Value::Tuple(xs) | Value::Parallel(xs) => {
                for x in xs {
                    self.node(x, Ctx::Other)?;
                }
                Ok(())
            }
            Value::If(c, t, e) => {
                self.node(c, Ctx::Other)?;
                self.node(t, ctx)?;
                self.node(e, ctx)
            }
            Value::Return(e) | Value::Yield(e) => self.node(e, Ctx::Other),
            Value::Drop(e) => self.node(e, Ctx::Discard),
            // A leaf, or a node that names no variable: every child-bearing variant is
            // named above (the list `Walk::node` keeps), so nothing here reaches the store.
            _ => Ok(()),
        }
    }
}
