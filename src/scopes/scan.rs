// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! The SCAN itself: the walk over a function body that opens and closes scopes, registers each
//! binding at its scope, and hands every node to the part that rewrites it.

use super::Scopes;
use super::backings::member_backing;
use super::disturbance::{disturbance_via, report_materialised_view};
use super::free_vars::Delivered;
use super::handoff::collect_drop_transferred;
use super::insert_free::scope_free_op_var;
use super::witness::witness_points_at;
use super::{Adopts, BindShape};
use crate::data::{Block, Data, Type, Value, v_set};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;

/// The binding a `text_mirror` block writes back: its one statement is
/// `OpSetText(subject, offset, Var(binding))`.
fn mirrored_binding(b: &Block) -> Option<u16> {
    let [Value::Call(_, args)] = b.operators.as_slice() else {
        return None;
    };
    match args.get(2).map(Value::unspan) {
        Some(Value::Var(v)) => Some(*v),
        _ => None,
    }
}

/// The variables a loop body assigns on EVERY pass: a `Set` among the loop's own statements, or
/// the statements of a block that is one of them, so no branch can skip it.  A body with a
/// `continue` anywhere in it answers none, since a `continue` before the `Set` skips it on that
/// pass.
fn loop_body_refills(lp: &Block) -> HashSet<u16> {
    let mut out = HashSet::default();
    let mut continues = false;
    for op in &lp.operators {
        op.walk(&mut |n| {
            if matches!(n.unspan(), Value::Continue(_)) {
                continues = true;
            }
        });
    }
    if continues {
        return out;
    }
    let mut take = |ops: &[Value]| {
        for op in ops {
            if let Value::Set(v, _) = op.unspan() {
                out.insert(*v);
            }
        }
    };
    take(&lp.operators);
    for op in &lp.operators {
        if let Value::Block(bl) = op.unspan() {
            take(&bl.operators);
        }
    }
    out
}

impl Scopes<'_> {
    /// The per-path flag of the store behind collection `args[0]` when this call GROWS it — an
    /// append or a new element — and a whole-collection copy may have moved its elements out
    /// (`collection_handoffs`); `None` otherwise.
    fn regrown_flag(
        &self,
        d_nr: u32,
        args: &[Value],
        function: &Function,
        data: &Data,
    ) -> Option<u16> {
        if d_nr != data.def_nr("OpAppendVector") && d_nr != data.def_nr("OpNewRecord") {
            return None;
        }
        let Value::Var(x) = args.first()?.unspan() else {
            return None;
        };
        function.tp(*x).depend().iter().find_map(|b| {
            if self.per_path_pairs.contains(&(u16::MAX, *b)) {
                self.handed_off.get(b).copied()
            } else {
                None
            }
        })
    }

    fn enter_scope(&mut self) -> u16 {
        self.stack.push(self.scope);
        self.scope = self.max_scope;
        self.max_scope += 1;
        self.scope
    }

    fn exit_scope(&mut self) {
        if let Some(scope) = self.stack.pop() {
            self.scope = scope;
        }
    }

    pub(super) fn scan(&mut self, val: &Value, function: &mut Function, data: &Data) -> Value {
        // loft#1320 — a KEYED branch value is not a `Set` RHS: `r = if … { g(hs) } else …`
        // lowers to `Set(r, Null)` and then `OpReplaceKeyed(if …, r, tp)`, which COPIES the
        // chosen arm into `r`'s own store and leaves the arm that MINTED with no owner.  The
        // branch is that op's first argument, so the per-arm rewrite `scan_set` applies to a
        // vector branch is applied here, with the temps homed where `r` lives.
        if let Some(rewritten) = self.rewrite_keyed_replace_branch(val, function, data) {
            return self.scan(&rewritten, function, data);
        }
        self.scan_depth += 1;
        assert!(
            self.scan_depth <= crate::limits::TREE_DEPTH,
            "expression nesting limit exceeded at depth {}",
            self.scan_depth
        );
        let result = self.scan_inner(val, function, data);
        self.scan_depth -= 1;
        result
    }

    /// The binding spelling of a group write resolved to its origin field, where that binding
    /// is materialised (loft#1664): `OpNewRecord(owner, T, f)` bound to an element of the
    /// binding becomes `OpNewRecord(binding, V, u16::MAX)`, and `OpFinishRecord(owner, elm, T,
    /// f)` the same — `V` being the collection type the binding's own spelling passes, recorded
    /// by the parser in `group_write_views`.  `None` for anything else, including a write whose
    /// destination already is the binding.
    fn respelled_group_write(
        &self,
        elm: Option<u16>,
        value: &Value,
        function: &Function,
        data: &Data,
    ) -> Option<Value> {
        let Value::Call(d, args) = value.unspan() else {
            return None;
        };
        let name = data.def(*d).name();
        let (elm, rest) = match (name, elm) {
            ("OpNewRecord", Some(e)) if args.len() == 3 => (e, &args[1..]),
            ("OpFinishRecord", None) if args.len() == 4 => match args[1].unspan() {
                Value::Var(e) => (*e, &args[2..]),
                _ => return None,
            },
            _ => return None,
        };
        let binding = function
            .tp(elm)
            .depend()
            .into_iter()
            .find(|b| function.group_write_views.contains_key(b))?;
        if !self.views_to_materialise.contains_key(&binding)
            || matches!(args[0].unspan(), Value::Var(x) if *x == binding)
            || rest.len() != 2
        {
            return None;
        }
        let plain = i32::from(function.group_write_views[&binding]);
        let mut out = args.clone();
        out[0] = Value::Var(binding);
        let n = out.len();
        out[n - 2] = Value::Int(plain);
        out[n - 1] = Value::Int(i32::from(u16::MAX));
        Some(Value::Call(*d, out))
    }

    #[expect(clippy::too_many_lines, reason = "inherited")]
    fn scan_inner(&mut self, val: &Value, function: &mut Function, data: &Data) -> Value {
        match val {
            // @FR-B-View — a `text` payload view MATERIALISES where its subject is disturbed
            // while it is still used: its value is the copy taken at the bind already, so what
            // ends is the write-through, which is the #673 mirror.  Dropped for that binding, at
            // every write, and reported as a record view's materialise is (loft#1665).
            Value::Block(b)
                if b.name == "text_mirror"
                    && let Some(v) = mirrored_binding(b)
                    && function.text_payload_views.contains(&v)
                    && let Some(cause) = self.views_to_materialise.get(&v).copied() =>
            {
                if self.text_views_reported.insert(v) {
                    let vname = function.name(v).to_string();
                    let cname = function.name(cause.container).to_string();
                    let fname = data.def(self.d_nr).original_name();
                    let via = disturbance_via(data, &cause);
                    report_materialised_view(
                        cause.reported_cause(),
                        &vname,
                        &cname,
                        &fname,
                        via.as_deref(),
                    );
                }
                Value::Null
            }
            Value::Var(ov) => Value::Var(*self.var_mapping.get(ov).unwrap_or(ov)),
            // @FR-B-View with @FR-Col-Group — a write through a group-member binding is spelled
            // against the origin field so it reaches the group's siblings; where the binding is
            // MATERIALISED that field is no longer the binding's, so the write is spelled back
            // to the binding's own copy (loft#1664).  Recognised by its element, whose type
            // records the binding, so an author's own `e.f += [r]` is never touched.
            Value::Set(elm, value)
                if let Some(rewritten) =
                    self.respelled_group_write(Some(*elm), value, function, data) =>
            {
                self.scan_set(*elm, &rewritten, function, data)
            }
            Value::Call(_, _)
                if let Some(rewritten) = self.respelled_group_write(None, val, function, data) =>
            {
                self.scan_inner(&rewritten, function, data)
            }
            Value::Set(ov, value) => self.scan_set(*ov, value, function, data),
            Value::Loop(lp) => {
                let scope = self.enter_scope();
                self.loops.push(scope);
                self.loop_refills.push(loop_body_refills(lp));
                function.mark_loop_scope(scope);
                // #316 — a loop body executes repeatedly: any ownership entry
                // the body touches is unreliable afterwards.  Keep only the
                // entries the body left unchanged.
                let owned_before = self.owned_refs.clone();
                let views_before = self.view_backing.clone();
                let backing_before = self.construction_backing.clone();
                let mints_before = self.tuple_call_mint.clone();
                let now_before = self.tuple_member_now.clone();
                // A loop body's hand-offs are armed BEFORE its statements are scanned, not in
                // scan order: on the next iteration an earlier statement displaces what a LATER
                // one handed off — `for … { x = mk(); x = p; }` reaches `x = mk()` holding the
                // parameter's copy — so the in-order fact would release a record the caller
                // owns.  Armed early, that displacement is suppressed on every iteration, which
                // loses the first iteration's release rather than doubling the later ones'.
                for op in &lp.operators {
                    let early = collect_drop_transferred(op, function, data, &self.per_path_pairs);
                    self.drop_transferred.extend(early);
                }
                let ls = self.convert(lp, function, data, false);
                // A local owned on entry to the loop AND at the end of its body is owned after
                // it, whether the body ran no passes or many, so it keeps its entry — at the
                // depth it had on entry.  Dropped for a body that REBOUND it (a changed depth),
                // a rebind after the loop released nothing for the last pass's record.  A local
                // whose body assignments mix owning and viewing releases through its owner
                // witness instead (loft#1336), which `displaced_drop` asks first.
                let owned_after_body = std::mem::replace(&mut self.owned_refs, owned_before);
                self.owned_refs
                    .retain(|k, _| owned_after_body.contains_key(k));
                self.view_backing
                    .retain(|k, b| views_before.get(k) == Some(b));
                self.construction_backing
                    .retain(|k, w| backing_before.get(k) == Some(w));
                self.tuple_call_mint
                    .retain(|k, m| mints_before.get(k) == Some(m));
                self.tuple_member_now
                    .retain(|k, m| now_before.get(k) == Some(m));
                self.loops.pop();
                self.loop_refills.pop();
                self.exit_scope();
                Value::Loop(Box::new(Block {
                    operators: ls,
                    result: Type::Void,
                    name: lp.name,
                    scope,
                    var_size: 0,
                }))
            }
            Value::If(test, t_val, f_val) => self.scan_if(test, t_val, f_val, function, data),
            Value::Break(lv) => {
                let mut ls = self.get_free_vars(
                    function,
                    data,
                    self.loops[self.loops.len() - *lv as usize - 1],
                    &Type::Void,
                    u16::MAX,
                    &Delivered::default(),
                );
                if ls.is_empty() {
                    Value::Break(*lv)
                } else {
                    ls.push(Value::Break(*lv));
                    Value::Insert(ls)
                }
            }
            Value::Continue(lv) => {
                let mut ls = self.get_free_vars(
                    function,
                    data,
                    self.loops[self.loops.len() - *lv as usize - 1],
                    &Type::Void,
                    u16::MAX,
                    &Delivered::default(),
                );
                if ls.is_empty() {
                    Value::Continue(*lv)
                } else {
                    ls.push(Value::Continue(*lv));
                    Value::Insert(ls)
                }
            }
            Value::Return(v) => {
                let expr = self.scan(v, function, data);
                Value::Insert(self.free_vars(
                    true,
                    &expr,
                    function,
                    data,
                    &data.def(self.d_nr).returned,
                    1,
                ))
            }
            Value::Block(bl) => {
                // pre-register a block-result Reference variable at the OUTER scope
                // before entering the block's inner scope.
                //
                // Without this, `scan_set(w, Null)` registers w at the inner scope.
                // At block exit, `free_vars` skips w (it is `ret_var`). At function exit,
                // `variables(outer_scope)` omits w (inner scope is not in the chain) →
                // OpFreeRef is never emitted → Database N not freed.
                //
                // Pre-registering at the outer scope causes `scan_set(w, Null)` inside the
                // block to see `var_scope.contains_key(&w) && *value == Null` → return
                // Insert([]) (the Set is suppressed from inside the block). We then hoist
                // Set(w, Null) to the outer level by returning Insert([Set(w,Null), Block]).
                //
                // This is necessary (not optional) because DbRef is 12 bytes (> 8) → Zone 2
                // of slot assignment handles it. Zone 2 of the outer scope's `process_scope`
                // walks its direct operators and finds Set(w, Null) in the Insert; Zone 2 of
                // the inner scope skips w (scope mismatch). If Set(w, Null) were left inside
                // the block, the outer Zone 2 would never see it and the slot would remain
                // u16::MAX → "variable never assigned a slot" panic at codegen.
                let mut hoisted_ref: Option<u16> = None;
                if let Some(Value::Var(orig_ret)) = bl.operators.last() {
                    let ret_v = *self.var_mapping.get(orig_ret).unwrap_or(orig_ret);
                    // @PLN164 B1 — a bind of the callee's minted record has its deps stripped
                    // by `scan_set` inside the block, after this decision, whether it ADOPTS
                    // or (under `LOFT_NO_ADOPT_FIRST_BIND`) copies; the parser's dep on the
                    // call's buffer is not a borrow (loft's inline container `f().pts[i]` over
                    // such a callee leaked one record per call — and again with the switch
                    // off while this asked the switched predicate, loft#1704).
                    let adopts = bl.operators.iter().any(|op| {
                        matches!(op.unspan(), Value::Set(w, value)
                        if w == orig_ret
                            && crate::use_analysis::binds_the_callees_minted_store(
                                data, function, ret_v, value,
                            ))
                    });
                    if !self.var_scope.contains_key(&ret_v)
                        && let Type::Reference(_, dep)
                        | Type::Vector(_, dep)
                        | Type::Enum(_, true, dep) = function.tp(ret_v)
                        && (dep.is_empty() || adopts)
                    {
                        // The hoisted null-init below stands right in front of the block, so
                        // the block's one bind is the temp's first (`deferred_first_bind`).
                        if adopts
                            && crate::keys::adopt_first_bind_enabled()
                            && !self.multi_assigned.contains(orig_ret)
                        {
                            function.mark_deferred_first_bind(ret_v);
                        }
                        self.var_scope.insert(ret_v, self.scope);
                        self.var_order.push(ret_v);
                        hoisted_ref = Some(ret_v);
                    }
                }
                // @FR-F-Call / @FR-F-Block — the TEXT twin of the hoist above.  A text value
                // block whose tail is a local first bound INSIDE it (`len({ s = "x{k}"; s })`,
                // and every `e ?? return|break|continue` over a text, whose `_ncr_N` temp is
                // that tail) hands its value out BY COPY: the reader appends or views the
                // bytes, nothing adopts the `String`.  Registered in the block, the local was
                // the block's `ret_var` — excluded from its exit frees — and no outer scope
                // knew it, so the interpreter never released it: one buffer per evaluation,
                // unbounded in a loop (loft#1907).  `--native` could not name it inside a
                // generator either, whose locals are state fields (loft#1937).
                //
                // Home it at the function body scope BEFORE the block is scanned, with its
                // init lifted to the root (the `__blk_N` hoist's home): every pass reuses the
                // one buffer and the function-exit sweep frees it once.  Registered first, so
                // an exit INSIDE the block (`?? continue` / `?? break`) does not free it on its
                // way out and leave the next pass, or the function exit, a second free.
                if hoisted_ref.is_none()
                    && self.scope >= 1
                    && matches!(bl.result.base(), Type::Text(_))
                    && let Some(Value::Var(orig_ret)) = bl.operators.last().map(Value::unspan)
                {
                    let ret_v = *self.var_mapping.get(orig_ret).unwrap_or(orig_ret);
                    if !self.var_scope.contains_key(&ret_v)
                        && !function.is_argument(ret_v)
                        && !function.is_skip_free(ret_v)
                        && matches!(function.tp(ret_v).base(), Type::Text(_))
                        && !self.lift_texts.contains(&ret_v)
                    {
                        self.var_scope.insert(ret_v, 1);
                        self.var_order.push(ret_v);
                        self.lift_texts.push(ret_v);
                    }
                }
                // The function body block (scope 0 → 1) with a non-void
                // result needs is_return=true so frees land between the
                // tail expression and the Return, not after it.
                let is_body_return = self.scope == 0
                    && bl.result != Type::Void
                    && data.def(self.d_nr).returned != Type::Void;
                let outer_scope = self.scope;
                let lift_watermark = self.lift_vars.len();
                let scope = self.enter_scope();
                // Move hoisted var from outer scope (0) to body scope so
                // get_free_vars at body exit can find and free it.
                if let Some(w) = hoisted_ref {
                    self.var_scope.insert(w, scope);
                }
                let mut ls = self.convert(bl, function, data, is_body_return);
                self.exit_scope();
                // loft#722 — a lift temp minted INSIDE a value-producing block must
                // outlive the block, because the block's result can be a borrow INTO
                // it.  `x = f().items[0] ?? Fallback {}` lowers the `??` to a block;
                // the temp holding `f()`'s result was registered at that block's
                // scope and freed on the way out, while `x` — a binding in the
                // ENCLOSING scope — still pointed into it.  It read correctly once
                // and returned zeroes after the store was reused.
                //
                // Without the `??` the same expression is lifted at statement level
                // and is correct, which is exactly the behaviour restored here:
                // re-register at the ENCLOSING scope and drop the block-exit free,
                // so the outer scope frees it instead.
                //
                // The enclosing scope, not the function: a lift inside a LOOP body
                // then still frees once per iteration, which is what keeps a loop
                // over such an expression flat instead of accumulating a store per
                // round.
                // Not for a BODY return.  The hoist hands the temp to the enclosing
                // scope and drops the block-exit free on the promise that the outer
                // scope frees it instead — and when the value block IS the function
                // body, the enclosing scope is that same function, so the free it was
                // handed to is the one just dropped and nothing frees it at all.
                // `fn txt(n: integer) -> text { mk(n).label }` leaked one record per
                // call for exactly that reason.  A body return also does not need the
                // hoist: the return delivers its value into the CALLER's buffer, so
                // freeing the temp at body exit — the behaviour before loft#722 — is
                // both correct and what the caller's copy relies on.
                if !matches!(bl.result, Type::Void)
                    && !is_body_return
                    && self.lift_vars.len() > lift_watermark
                {
                    let fresh: Vec<u16> = self.lift_vars[lift_watermark..]
                        .iter()
                        .copied()
                        .filter(|v| self.var_scope.get(v) == Some(&scope))
                        .collect();
                    // Only the temps the RESULT actually borrows from.  Hoisting
                    // every lift in a value block moves frees that nothing was
                    // waiting on, and those then went missing entirely (27 leaked
                    // `File` stores in the file suite) — the block, not the outer
                    // scope, is the right owner when the result does not point into
                    // the temp.
                    let borrowed = Self::result_borrow_roots(&ls, data);
                    let hoisted: Vec<u16> =
                        fresh.into_iter().filter(|v| borrowed.contains(v)).collect();
                    for v in &hoisted {
                        self.var_scope.insert(*v, outer_scope);
                    }
                    if !hoisted.is_empty() {
                        ls.retain(|op| {
                            scope_free_op_var(op, data).is_none_or(|v| !hoisted.contains(&v))
                        });
                    }
                }
                let block = Value::Block(Box::new(Block {
                    operators: ls,
                    result: bl.result.clone(),
                    name: bl.name,
                    scope,
                    var_size: 0,
                }));
                if let Some(w) = hoisted_ref {
                    // Return Insert([Set(w, Null), Block]) so that:
                    // 1. Zone-2 slot assignment sees Set(w, Null) at the outer scope level.
                    // 2. get_free_vars at the outer scope emits OpFreeRef(w) on block exit.
                    Value::Insert(vec![v_set(w, Value::Null), block])
                } else {
                    block
                }
            }
            Value::Call(d_nr, args) => {
                // loft#1510 / D-heap-4 — an IN-PLACE literal rebuild (`parse_object`'s
                // in-place arm) reaches the scan as a bare `OpDatabase` on the local, never
                // as a `Set`, so the owner witness would keep naming the store this clears
                // and the record's hook would be lost at the overwrite.  Run the hook first
                // — by identity, through the witness, which holds the sentinel whenever the
                // local does not own its store — and re-point the witness after: the rebuilt
                // record is a store the local mints.  The store itself is reused in place,
                // so nothing is freed here.
                if *d_nr == data.def_nr("OpDatabase")
                    && let Some(Value::Var(ov0)) = args.first().map(Value::unspan)
                {
                    let v = *self.var_mapping.get(ov0).unwrap_or(ov0);
                    if let Some(&w) = self.owner_witness.get(&v) {
                        let mut ops = Vec::new();
                        if let Some(hook) = self.witness_hook(function, data, v, w) {
                            ops.push(hook);
                        }
                        ops.push(v_set(
                            w,
                            Value::Call(data.def_nr("OpNullRefSentinel"), vec![]),
                        ));
                        let mut call_args = vec![Value::Var(v)];
                        call_args.extend(args.iter().skip(1).cloned());
                        ops.push(Value::Call(*d_nr, call_args));
                        ops.push(witness_points_at(w, v, data));
                        // The rebuilt record is the local's own, whatever it held before, and a
                        // rebuild is no `Set`, so the `@FR-O-Latest` retire a `Set` writes is
                        // written here: left set by an earlier parameter copy, scope exit would
                        // skip this record's hook.
                        if let Some(&flag) = self.handed_off.get(&v) {
                            ops.push(v_set(flag, Value::Boolean(false)));
                        }
                        return Value::Insert(ops);
                    }
                }
                // `@FR-L-CapOwn`, `@FR-O-Witness` — a capture written into a closure record hands
                // the local's store to the record: the witness stops naming it, so a later
                // reassignment of the local does not free what the record holds.  Here, at the
                // write, and not at the statement around it: a record built in place in a
                // struct's field (loft#1867) is no `Set` of a fn-ref local.
                if *d_nr == data.def_nr("OpSetDbRef")
                    && let (Some(Value::Var(rec)), Some(Value::Var(c))) = (
                        args.first().map(Value::unspan),
                        args.get(2).map(Value::unspan),
                    )
                    && function
                        .name(*self.var_mapping.get(rec).unwrap_or(rec))
                        .starts_with("___clos_")
                    && let Some(&cw) = self.owner_witness.get(self.var_mapping.get(c).unwrap_or(c))
                {
                    let (mut ops, ls, postamble) = self.scan_args(args, function, data, *d_nr);
                    ops.push(Value::Call(*d_nr, ls));
                    ops.extend(postamble);
                    ops.push(v_set(
                        cw,
                        Value::Call(data.def_nr("OpNullRefSentinel"), vec![]),
                    ));
                    return Value::Insert(ops);
                }
                let (preamble, ls, postamble) = self.scan_args(args, function, data, *d_nr);
                let call = Value::Call(*d_nr, ls);
                // D-heap-14 — a copy that hands a droppable over inside a branch ARM records that
                // it RAN on the source's per-path flag (loft#1515), right after it: the source
                // keeps its release on every path where this copy did not run.
                let flags = self.arm_handoff_flags(*d_nr, args, function, data);
                if !flags.is_empty() {
                    let mut ops = preamble;
                    ops.push(call);
                    ops.extend(postamble);
                    for flag in flags {
                        ops.push(v_set(flag, Value::Boolean(true)));
                    }
                    return Value::Insert(ops);
                }
                // A collection whose elements a copy moved out, GROWN again: its store holds
                // elements it must release once more.  `(H-Spent)` refuses the program — the name
                // was spent — and until that error exists the release is given back, which keeps
                // the answer such a program had before the move was honoured (D-heap-23).  Ahead
                // of the call, because `OpNewRecord` answers the element it adds.
                let regrown = self.regrown_flag(*d_nr, args, function, data);
                let call = match regrown {
                    Some(flag) => Value::Insert(vec![v_set(flag, Value::Boolean(false)), call]),
                    None => call,
                };
                if preamble.is_empty() && postamble.is_empty() {
                    call
                } else if postamble.is_empty() {
                    let mut ops = preamble;
                    ops.push(call);
                    Value::Insert(ops)
                } else {
                    // @PLN90 / loft#506 — run the store-back postamble AFTER the call.  For a
                    // non-void call, capture the result into a temp so the Insert still yields
                    // the CALL's value; a scalar must NOT be inline-ref (freeing a scalar-as-ref
                    // corrupts the store), so use a plain slotted temp.
                    let mut ops = preamble;
                    let ret = data.def(*d_nr).returned.clone();
                    if ret == Type::Void {
                        ops.push(call);
                        ops.extend(postamble);
                    } else {
                        let is_scalar = matches!(
                            ret,
                            Type::Integer(..)
                                | Type::Float
                                | Type::Single
                                | Type::Boolean
                                | Type::Character
                        );
                        let rtmp = if is_scalar {
                            self.lift_counter += 1;
                            let name = format!("__wbret_{}", self.lift_counter);
                            let t = function.add_temp_var(&name, &ret);
                            self.var_scope.insert(t, self.scope);
                            t
                        } else {
                            self.new_lift_var(function, &ret)
                        };
                        ops.push(self.lift_set(rtmp, call, function, data));
                        ops.extend(postamble);
                        ops.push(Value::Var(rtmp));
                    }
                    Value::Insert(ops)
                }
            }
            Value::CallRef(v_nr, args) => {
                let (preamble, ls, _postamble) = self.scan_args(args, function, data, u32::MAX);
                // The CALLEE slot is a READ of the variable, so it is remapped like every
                // other var-carrying node (`Var`, `TupleGet`, `FnRefDnr`, … below).  A `Set`
                // to a name whose block has ended starts a NEW binding (`@FR-B-Scope`) with a
                // slot of its own (`scan_set` → `copy_variable`), and a call through the name
                // must reach that binding, never the ended one (loft#1679).  `--native` names
                // its locals `var_<name>`, so both bindings share one Rust local there and a
                // missing remap is invisible: the interpreter is where it shows.
                let call = Value::CallRef(*self.var_mapping.get(v_nr).unwrap_or(v_nr), ls);
                if preamble.is_empty() {
                    call
                } else {
                    let mut ops = preamble;
                    ops.push(call);
                    Value::Insert(ops)
                }
            }
            Value::Insert(ops) => {
                Value::Insert(ops.iter().map(|v| self.scan(v, function, data)).collect())
            }
            Value::Drop(inner) => {
                let scanned = self.scan(inner, function, data);
                // #490 — a discarded statement result that owns a fresh store
                // (`json_parse(x);`, `mk();`) lowers to a plain stack-pop
                // (`FreeStack` on the interpreter, a dropped Rust return value
                // on native), which never frees the store — once per iteration
                // inside a loop.  Bind it to a `__lift_N` temp instead, so
                // `get_free_vars` emits the store's `OpFreeRef` at scope exit —
                // the same machinery `scan_args` uses for owned call-argument
                // temps.  A `Set` consumes the value, so the `Drop` wrapper is
                // dropped with it.
                if let Value::Insert(mut ops) = scanned {
                    // A call whose arguments were themselves lifted arrives as
                    // `Insert([Set(__lift_i, …)…, call])` — the owned result is
                    // the final op.
                    if let Some(last) = ops.last()
                        && let Some(tp) = self.inline_struct_return(last, data, u32::MAX, function)
                    {
                        let tmp = self.new_lift_var(function, &tp);
                        let last = ops.pop().unwrap();
                        ops.push(self.lift_set(tmp, last, function, data));
                        Value::Insert(ops)
                    } else {
                        Value::Drop(Box::new(Value::Insert(ops)))
                    }
                } else if let Some(tp) =
                    self.inline_struct_return(&scanned, data, u32::MAX, function)
                {
                    let tmp = self.new_lift_var(function, &tp);
                    self.lift_set(tmp, scanned, function, data)
                } else {
                    Value::Drop(Box::new(scanned))
                }
            }
            Value::Iter(idx, create, next, extra) => {
                let scanned_create = self.scan(create, function, data);
                // #316 — `next`/`extra` execute once per iteration: drop any
                // ownership entry they touch (same rationale as Value::Loop).
                let owned_before = self.owned_refs.clone();
                let views_before = self.view_backing.clone();
                let backing_before = self.construction_backing.clone();
                let mints_before = self.tuple_call_mint.clone();
                let now_before = self.tuple_member_now.clone();
                let scanned_next = self.scan(next, function, data);
                let scanned_extra = self.scan(extra, function, data);
                self.owned_refs
                    .retain(|k, depth| owned_before.get(k) == Some(depth));
                self.view_backing
                    .retain(|k, b| views_before.get(k) == Some(b));
                self.construction_backing
                    .retain(|k, w| backing_before.get(k) == Some(w));
                self.tuple_call_mint
                    .retain(|k, m| mints_before.get(k) == Some(m));
                self.tuple_member_now
                    .retain(|k, m| now_before.get(k) == Some(m));
                Value::Iter(
                    *idx,
                    Box::new(scanned_create),
                    Box::new(scanned_next),
                    Box::new(scanned_extra),
                )
            }
            Value::Tuple(elems) => {
                Value::Tuple(elems.iter().map(|v| self.scan(v, function, data)).collect())
            }
            Value::TupleGet(var, idx) => {
                Value::TupleGet(*self.var_mapping.get(var).unwrap_or(var), *idx)
            }
            Value::TuplePut(var, idx, inner) => {
                let v = *self.var_mapping.get(var).unwrap_or(var);
                let value = self.scan(inner, function, data);
                self.member_write(v, *idx, value, function, data)
            }
            // @PLAN53 cluster 2: remap the var-numbers these IR nodes carry through
            // `var_mapping` — exactly like `Var`/`TupleGet`/`TuplePut` above.  They
            // were missing, so when a sibling scope reuses a name (`copy_variable`),
            // a fn-ref loop break-test (`FnRefDnr`) or a copied closure capture
            // (`FnRef.clos_var`) kept pointing at the ORIGINAL var.  V1 masked it (the
            // original + copy share a slot); V2 gives them distinct slots, so the
            // stale read hit the wrong slot (repro_p352: 2nd reused-name fn-ref loop
            // read loop-1's exhausted sentinel → 0).  `clos_var == u16::MAX`
            // (non-capturing) is left untouched — `var_mapping` never holds u16::MAX.
            Value::FnRefDnr(var) => Value::FnRefDnr(*self.var_mapping.get(var).unwrap_or(var)),
            Value::FnRef(d_nr, clos_var, fn_type) => Value::FnRef(
                *d_nr,
                *self.var_mapping.get(clos_var).unwrap_or(clos_var),
                fn_type.clone(),
            ),
            Value::Yield(inner) => Value::Yield(Box::new(self.scan(inner, function, data))),
            Value::Span(b) => {
                let scanned = self.scan(&b.1, function, data);
                // When scanning lifted an inline struct-returning-call argument
                // (@P297), the result is `Insert([Set(__lift_N, …), final])` — a
                // statement sequence, not a positioned expression.  Re-wrapping
                // it in a Span hides the lift preamble from the consumers that
                // hoist it to statement level (`scan_set`'s flatten and
                // `scan_args`'s `is_p135_hoisted` bubbling, both `if let
                // Value::Insert`).  The interpreter tolerates the hidden Insert;
                // the native backend would emit `Set(__lift_N, …)` inside an
                // enclosing expression and fail to compile.  Inner ops keep
                // their own positions, so dropping the outer span is safe.
                //
                // SURGICAL: only unwrap when the Insert's leading op is a lift
                // `Set(__lift_N, …)`.  Other span-wrapped Inserts (closure-record
                // construction, etc.) MUST keep their span — unwrapping them
                // broadly breaks closure-in-struct-field construction (`invalid
                // fn-ref` in native codegen, @P258/@P259 territory).
                //
                // The A5.6 null-init preamble ([`Self::is_null_init_preamble`]) is the
                // second shape that must survive to `scan_args`, and it is just as
                // narrow: exactly two ops, led by an owned-heap `Set(v, Null)`.
                let is_lift_preamble = matches!(&scanned, Value::Insert(ops)
                    if ops.first().is_some_and(|op| matches!(op,
                        Value::Set(v, _) if function.name(*v).starts_with("__lift_")))
                        || Self::is_null_init_preamble(ops, function));
                if is_lift_preamble {
                    scanned
                } else {
                    Value::with_span(b.0, scanned)
                }
            }
            _ => {
                // EVERY node that carries a variable index needs an arm above, because a
                // split binding (`scan_set`'s `copy_variable`) gives the second binding its
                // own slot and the index this walk copies through is the FIRST one's.  The
                // enumeration is what keeps failing — `FnRefDnr`/`FnRef` were added one
                // release after `Var`/`TupleGet`, `CallRef` one after those (loft#1679) —
                // so name the set here: dropping an arm is then a failed assertion rather
                // than a wrong slot read on one backend.
                debug_assert!(
                    !matches!(
                        val,
                        Value::Var(_)
                            | Value::Set(..)
                            | Value::CallRef(..)
                            | Value::TupleGet(..)
                            | Value::TuplePut(..)
                            | Value::FnRef(..)
                            | Value::FnRefDnr(_)
                            | Value::Iter(..)
                    ),
                    "scan: a var-carrying node reached the pass-through arm, so its variable                      index escapes `var_mapping`"
                );
                val.clone()
            }
        }
    }

    /// Register `v`'s scope.  Normally the current scope, but on the gated
    /// phase-2 re-scan a confined `__vdb`/local registers at its block scope
    /// (plan-57 cluster I) so the block-exit `free_vars` sweep frees its store
    /// there instead of at function exit.  `confined` is empty on phase 1, so
    /// this is identical to `var_scope.insert(v, self.scope)` in the common case.
    pub(super) fn put_scope(&mut self, v: u16) {
        let scope = self.confined.get(&v).copied().unwrap_or(self.scope);
        self.var_scope.insert(v, scope);
    }

    /// Register `v` at the current scope, in its declaration turn.
    pub(super) fn register_binding(&mut self, v: u16, function: &Function) {
        self.put_scope(v);
        // `(H-Drop)` releases at a scope's end in REVERSE DECLARATION order.  A collection
        // local is a view of the store that holds its elements — its `__vdb_N` backing, or
        // the `__ref_N` buffer a call delivered it through — and that store is registered by
        // its null-init at the head of the function, so its release came after every other
        // local's, whatever the order they were declared in (D-heap-21).  The store is minted
        // where the local is bound, so its place in the sweep is there.  A RECORD local
        // releases through itself, and its buffer's free is guarded by identity, so a record
        // buffer keeps the place it has.  A TUPLE local releases through its members'
        // backings — a vector member's `__vdb_N`, a record member a whole-tuple bind copied
        // into a `__ref_N` (loft#1361) — so those take its place too (loft#1588).
        let collection = matches!(function.tp(v).base(), Type::Vector(_, _) | Type::Tuple(_));
        // A fn-ref's closure record releases in the fn-ref's turn, just BEFORE it (loft#1606):
        // the record's release runs its cascade and then frees its store, where the fn-ref's
        // free alone frees the store with no cascade — so run first, it left the record's
        // cascade to read a freed record.  The fn-ref's free after it finds the store gone.
        let fn_ref = matches!(function.tp(v).base(), Type::Function(..));
        let mut records = Vec::new();
        let mut backings = function.tp(v).depend().clone();
        if let Type::Tuple(elems) = function.tp(v).base() {
            backings.extend(elems.iter().filter_map(|e| member_backing(function, e)));
        }
        for d in backings {
            let name = function.name(d);
            if fn_ref
                && name.starts_with("___clos_")
                && let Some(pos) = self.var_order.iter().position(|&x| x == d)
            {
                self.var_order.remove(pos);
                records.push(d);
            } else if (name.starts_with("__vdb_") || (collection && name.starts_with("__ref_")))
                && let Some(pos) = self.var_order.iter().position(|&x| x == d)
            {
                self.var_order.remove(pos);
                self.var_order.push(d);
            }
        }
        self.var_order.push(v);
        self.var_order.extend(records);
    }

    #[must_use]
    pub(super) fn variables(&self, to_scope: u16) -> Vec<u16> {
        let mut scopes = HashSet::default();
        let mut sc = self.scope;
        let mut scope_pos = self.stack.len();
        loop {
            if sc == 0 {
                // never return function arguments
                break;
            }
            scopes.insert(sc);
            if sc == to_scope {
                break;
            }
            if scope_pos == 0 {
                break;
            }
            scope_pos -= 1;
            sc = self.stack[scope_pos];
        }
        // Iterate var_order in reverse (most-recently-inserted first) so that
        // OpFreeRef/OpFreeText are emitted in reverse-allocation order, satisfying
        // the LIFO invariant enforced by database::free().
        let mut res = Vec::new();
        for &v_nr in self.var_order.iter().rev() {
            if let Some(sc) = self.var_scope.get(&v_nr)
                && scopes.contains(sc)
                && !self.binding_now.contains(&v_nr)
            {
                res.push(v_nr);
            }
        }
        res
    }

    /// A `__lift_N` that OWNS a vector store for the function's whole life — a per-site
    /// buffer like the parser's `__vdb_N`: allocated by its function-entry `Set(tmp, Null)`
    /// (an owned vector's null-init allocates), refilled IN PLACE by the arm that reaches it,
    /// and freed once at function exit.  Homed at the function's root scope for exactly that
    /// reason: freed per iteration, it would be refilled dead.
    pub(super) fn new_buffer_var(&mut self, function: &mut Function, tp: &Type) -> u16 {
        self.lift_counter += 1;
        let name = format!("__lift_{}", self.lift_counter);
        let tmp = function.add_temp_var(&name, tp);
        let root = self.stack.get(1).copied().unwrap_or(self.scope);
        self.lift_decl_depth.insert(tmp, 0);
        self.var_scope.insert(tmp, root);
        self.var_order.push(tmp);
        self.lift_vars.push(tmp);
        tmp
    }

    /// Create a `__lift_N` temporary that OWNS an inline call result, so
    /// `get_free_vars` emits its `OpFreeRef` at scope exit.  Registers the
    /// var in the current scope and in `lift_vars` (which drives the
    /// function-entry `Set(v, Null)` slot reservation).  The caller emits
    /// the `Set(tmp, call)` itself — as an arg preamble (`scan_args`) or as
    /// the statement replacing a `Drop` (#490).
    /// The `__ref_N` / `__rref_N` buffers a call hands to a loft-defined callee, paired with
    /// the variable `v` its result is bound to — the identity-guarded frees `get_free_vars`
    /// emits (`OpFreeRefIfDistinct`) where the two may alias at run time.  One home for the
    /// named bind (`scan_set`) and the lifted one (`lift_set`): the pairing is a property of
    /// the call and its destination, not of how the destination was spelled.
    pub(super) fn pair_call_buffers(
        &mut self,
        v: u16,
        unspanned_value: &Value,
        function: &Function,
        bind: BindShape,
    ) {
        let BindShape {
            adopts,
            publishes_through_ref,
            vector_shaped,
        } = bind;
        let adopts_fresh_store = adopts == Adopts::Fresh;
        let adopts_minted = adopts == Adopts::Minted;
        if (adopts_fresh_store || adopts_minted || publishes_through_ref || vector_shaped)
            && let Value::Call(_, args) = unspanned_value
        {
            for arg in args {
                let arg_var = match arg.unspan() {
                    Value::Var(av) => Some(*av),
                    Value::Set(av, _) => Some(*av),
                    _ => None,
                };
                if let Some(av) = arg_var {
                    let n = function.name(av);
                    if n.starts_with("__ref_") || n.starts_with("__rref_") {
                        // `av`'s scope is inherited from the enclosing
                        // assignment: `self.scope`.  `v`'s scope was
                        // just written above.  Only pair when the
                        // witness `v` lives AT LEAST as long as
                        // `av` — i.e. `var_scope[v] <= var_scope[av]`.
                        // Otherwise, when codegen lowers the function
                        // to Rust, the witness's `let` falls out of
                        // its block scope before `av`'s OpFreeRef
                        // fires, and the emitted `var_f.store_nr`
                        // references a dead name (e.g. `f = file(…,
                        // __ref_1)` inside a nested `{}` block).
                        //
                        // loft#759 — a PARAMETER has no such block to fall
                        // out of: its `let` is the function signature, so
                        // it outlives every local including `av`, on both
                        // backends.  Its `var_scope` entry is written when
                        // the body first assigns it, which for a set inside
                        // an `if` reads as INNER-scoped and would route a
                        // valid witness into the @P378(a) branch below.
                        let av_scope = self.var_scope.get(&av).copied().unwrap_or(u16::MAX);
                        let v_scope = self.var_scope.get(&v).copied().unwrap_or(u16::MAX);
                        // A buffer is never its own witness.  The pairing exists to
                        // skip the buffer's free when ANOTHER variable adopted its
                        // store; `__ref_N = f(__ref_N)` has no other variable, and
                        // the guard then compares the store with itself and never
                        // frees at all — the buffer's own store leaks (loft#1013,
                        // where capturing the call's answer into the buffer it was
                        // handed is what gives the value an owner).
                        if av == v {
                            continue;
                        }
                        if adopts_minted && !adopts_fresh_store {
                            self.minted_pairs.insert(av);
                        }
                        // A vector admitted here ONLY by the alias case below
                        // (`!adopts_fresh_store`, no `&`) takes the inner-slot branch
                        // and nothing else.  Making the BUFFER's own free conditional
                        // on the slot is the opposite trade and is wrong for it: the
                        // slot may have no free of its own, and then neither store is
                        // released.  Measured — widening both branches leaked across
                        // sixteen suites (loft#1201).
                        let vector_alias_only =
                            vector_shaped && !adopts_fresh_store && !publishes_through_ref;
                        if !vector_alias_only
                            && ((publishes_through_ref && function.is_argument(v))
                                || (v_scope <= av_scope && v_scope != u16::MAX))
                        {
                            self.paired_witness.entry(av).or_insert(v);
                        } else if v_scope != u16::MAX && av_scope != u16::MAX && v_scope > av_scope
                        {
                            // @P378(a) — witness `v` is INNER-scoped (e.g.
                            // a loop body) while the `__ref_N` buffer `av`
                            // is OUTER (function).  The buffer is reserved
                            // once but `v` (which adopts the buffer's
                            // store) is freed every iteration; that frees
                            // the buffer's store, which `find_free_slot`
                            // then recycles to a callee temp next
                            // iteration — two OpDatabase targets collide on
                            // one record (self-referential keyed insert →
                            // SIGSEGV).  Make `v`'s per-iteration free
                            // conditional on NOT aliasing the buffer:
                            // adoption → skip (store stays reserved, freed
                            // once by the buffer's function-exit OpFreeRef);
                            // fresh-store → real free.  Scope-safe for
                            // native because `av` (outer) outlives `v`.
                            let buffers = self.witness_buffer.entry(v).or_default();
                            if !buffers.contains(&av) {
                                buffers.push(av);
                            }
                        }
                    }
                }
            }
        }
    }

    /// `Set(tmp, call)` for a `__lift_N` temp — the bind of a call result the argument
    /// scan LIFTED out of its call (`use(mk(n), …)`).
    ///
    /// The temp is null-initialised in the function prologue (`lift_vars`), so its one bind
    /// here read as a REBIND, and a rebind of a record from a callee whose return carries its
    /// buffer's dep copies where a first bind adopts (`@FR-O-Move`): `pa_text(pa_decode(f),
    /// "op")` deep-copied the decoded tree into a store minted for the copy and freed the
    /// callee's, while `d = pa_decode(f); pa_text(d, "op")` took the callee's store as it
    /// was — 14 % of `check_request`'s profile in one `OpCopyRecord`.  The bind after the
    /// prologue's null-init is the temp's first on every path (`deferred_first_bind`, the
    /// fact @PLN164 B1 records for a local behind an `if` pre-init), and the buffer is paired
    /// for the identity-guarded free exactly as the named bind's is (`pair_call_buffers`).
    ///
    /// Only the callee shape that COPIED changes: a fresh (dep-empty) return already adopted
    /// at the rebind, a borrowed view is never lifted, and a temp `mark_lift_handoff` moved
    /// into its callee keeps the ownership the move relies on — the temp still owns one store
    /// exclusively, the callee's mint instead of a copy of it.
    pub(super) fn lift_set(
        &mut self,
        tmp: u16,
        value: Value,
        function: &mut Function,
        data: &Data,
    ) -> Value {
        let unspanned = value.unspan();
        if let Value::Call(fn_nr, _) = unspanned
            && (*fn_nr as usize) < data.definitions.len()
            && data.def(*fn_nr).is_loft_defined()
            && !data.def(*fn_nr).return_adopts_fresh_store()
            && crate::use_analysis::adopts_minted_at_bind(data, function, tmp, unspanned)
        {
            function.mark_deferred_first_bind(tmp);
            // A temp a copy CONSUMES (`keep += [mk(i)]`: the element copy's source-free bit
            // releases the temp's store, `mark_lift_handoff`) has no free of its own to guard,
            // so the buffer is not paired: pairing it in a loop enrolls it in the record-buffer
            // pool (`reuse_record_buffers` reads `witness_buffer`), and the consuming copy then
            // released the pooled store every turn — `164-forward-tuple`'s q6 read a freed
            // record on both backends.  Unpaired, the buffer stays null and the callee mints
            // per call, which the copy releases: the shape the named form has had all along.
            if self.drop_transferred.contains(&tmp) || self.free_transferred.contains(&tmp) {
                return v_set(tmp, value);
            }
            self.pair_call_buffers(
                tmp,
                unspanned,
                function,
                BindShape {
                    adopts: Adopts::Minted,
                    publishes_through_ref: false,
                    vector_shaped: false,
                },
            );
        }
        v_set(tmp, value)
    }

    pub(super) fn new_lift_var(&mut self, function: &mut Function, tp: &Type) -> u16 {
        self.lift_counter += 1;
        let name = format!("__lift_{}", self.lift_counter);
        let tmp = function.add_temp_var(&name, tp);
        function.mark_inline_ref(tmp);
        let witness = self.pending_join_witness.replace(u16::MAX);
        if witness != u16::MAX {
            self.lift_join_witness.insert(tmp, witness);
        }
        self.lift_decl_depth.insert(tmp, self.loops.len());
        self.var_scope.insert(tmp, self.scope);
        self.var_order.push(tmp);
        self.lift_vars.push(tmp);
        tmp
    }

    /// loft#722 — the variables a block's RESULT may point INTO.
    ///
    /// `OpGetField` / `OpGetVector` / `OpGetEnum` read into their first argument's
    /// record, so a chain of them still points at the variable the chain starts
    /// from — the same "walk the getters to the root" fact loft#666 needed for a
    /// `match` subject. A chain rooted in a CALL produces a value of its own and
    /// roots nothing.
    ///
    /// The result of a `??` block is a variable assigned EARLIER in the block
    /// (`__ncc_N = OpGetVectorNullable(OpGetField(tmp, …))`), so a var is resolved
    /// through its in-block assignment before being reported.
    fn result_borrow_roots(ops: &[Value], data: &Data) -> HashSet<u16> {
        // var -> what its assignment points into, for Sets seen in this block.
        let mut from: HashMap<u16, u16> = HashMap::default();
        for op in ops {
            if let Value::Set(v, rhs) = op.unspan()
                && let Some(root) = Self::borrow_root(rhs, data)
            {
                from.insert(*v, root);
            }
        }
        // The result is the last op that is not a scope-exit free; take every var
        // it could evaluate to (both arms of an `if`, etc.).
        let Some(result) = ops
            .iter()
            .rev()
            .find(|o| scope_free_op_var(o, data).is_none())
        else {
            return HashSet::default();
        };
        let mut roots = HashSet::default();
        result.walk(&mut |n| {
            if let Some(r) = Self::borrow_root(n, data) {
                let mut cur = r;
                // Follow the in-block assignment chain, bounded by its own size so
                // a cycle cannot spin.
                for _ in 0..=from.len() {
                    roots.insert(cur);
                    match from.get(&cur) {
                        Some(next) if *next != cur => cur = *next,
                        _ => break,
                    }
                }
            }
        });
        roots
    }

    /// The variable a value points INTO, following getter chains; `None` when it
    /// produces a value of its own.
    fn borrow_root(val: &Value, data: &Data) -> Option<u16> {
        match val.unspan() {
            Value::Var(v) => Some(*v),
            // loft#722 — a getter roots the chain only when it RETURNS A BORROW, and
            // the stdlib declaration already says which do: `OpGetField(v1, fld) ->
            // reference[v1]` and `OpGetVector(r, …) -> reference[r]` name the argument
            // they read into, while `OpGetInt(v1, fld) -> integer` names nothing
            // because it copies a scalar out.  Read that declared dep instead of the
            // `OpGet` name prefix: the prefix is a proxy, and it was too wide.
            //
            // `run() -> integer { make2().n }` lowers to `OpGetInt(__lift_1, 0)`, whose
            // result is an integer that cannot point into the temp. Treating it as a
            // borrow hoisted `__lift_1` to the enclosing scope and dropped its
            // block-exit free, so the call's store was never freed — one leaked record
            // per inline struct-returning call, on both backends.
            //
            // Every borrowing getter names its FIRST argument, so the walk itself is
            // unchanged; only which calls enter it.
            Value::Call(d, args)
                if data.def(*d).name().starts_with("OpGet")
                    && matches!(
                        data.def(*d).returned.base(),
                        Type::Reference(_, _)
                            | Type::Vector(_, _)
                            | Type::Text(_)
                            | Type::Enum(_, true, _)
                            | Type::Sorted(_, _, _)
                            | Type::Hash(_, _, _)
                            | Type::Index(_, _, _)
                    ) =>
            {
                args.first().and_then(|a| Self::borrow_root(a, data))
            }
            _ => None,
        }
    }
}
