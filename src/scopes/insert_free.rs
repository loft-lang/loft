// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Placing the releases around a block's VALUE: frees go before a tail that does not read what
//! they free, into the arms of a branch, and copied text sources are released after the copy.

use super::returns::{collect_return_sources, is_value_return_type};
use super::{Scopes, call};
use crate::data::{Block, Data, Type, Value, v_set};
use crate::fxhash::FxHashSet as HashSet;
use crate::variables::Function;

/// After a return value has been COPIED — into the caller's hidden `&text` buffer, or into
/// a `__ret_N` temp — the frame-local text temps it read are dead: the caller consumes
/// the copy, not them.  Two kinds are otherwise never freed on this path, because each
/// was suppressed on the premise that the return would TRANSFER it rather than copy it:
///
/// - a `__work_N` text that `wrap_value_text_dest` minted for a callee to fill — as the
///   return's terminal it is skipped by `get_free_vars` (`ret_var`);
/// - a `__ncc_N` null-coalesce temp, `skip_free` so the present-path Str outlives its
///   block; a NON-tail consumer frees it in place (`collect_consumed_ncc_text`), and a
///   return that copies is exactly such a consumer.
///
/// Emit the frees HERE, at the copy, so they fire only when a copy actually happened; the
/// direct-transfer path (fast-path `Return(Var(__work_N))`, no copy) reaches neither this
/// nor a free and correctly leaves the buffer for the caller.  An argument is the
/// caller's.  A user local is freed by the scope sweep already — EXCEPT the one the
/// return names: the sweep suppresses the returned variable on the premise that it is
/// handed up, and once its bytes are copied into the buffer that premise is false, so a
/// `return ta` delivered through the buffer frees `ta` here too (loft#1357; a lambda
/// holding its one buffer for `tb` returned `ta` as a view of an orphan).
pub(super) fn free_copied_text_sources(
    result: &mut Vec<Value>,
    expr: &Value,
    pending: &[Value],
    function: &Function,
    data: &Data,
) {
    let mut srcs = Vec::new();
    collect_return_sources(expr, data, &mut srcs);
    for w in srcs {
        if !matches!(function.tp(w).base(), Type::Text(_)) || function.is_argument(w) {
            continue;
        }
        // A source the scope-exit sweep already releases is not drained twice: a
        // multi-arm value has no single returned var for `get_free_vars` to suppress,
        // so a `__work_N` behind a formatted-string ARM sits in `pending` as well.
        if pending
            .iter()
            .any(|op| scope_free_op_var(op, data) == Some(w))
        {
            continue;
        }
        let n = function.name(w);
        if n.starts_with("__work_")
            || (n.starts_with("__ncc_") && function.is_skip_free(w))
            || (!function.is_skip_free(w) && !matches!(function.tp(w), Type::RefVar(_)))
        {
            result.push(call("OpFreeText", w, data));
        }
    }
}

/// ANY hidden `&text` return buffer of `d_nr`, read by the value or not — the destination a
/// STAGED return moves into (`text_return_buffer_for` is the direct-write question, which
/// must exclude a buffer the value reads).  `None` = the function holds no buffer at all.
pub(super) fn any_text_return_buffer(function: &Function, data: &Data, d_nr: u32) -> Option<u16> {
    data.def(d_nr)
        .attributes()
        .iter()
        .filter(|a| {
            a.hidden && matches!(&a.typedef, Type::RefVar(t) if matches!(**t, Type::Text(_)))
        })
        .map(|a| function.var(&a.name))
        .find(|&v| v != u16::MAX)
}

/// The hidden `&text` return buffer of `d_nr` that `expr` does not read — the caller-owned
/// destination an owned text return is delivered through (@FR-F-Ret), as a variable of
/// the function's own frame.
///
/// A function holds one such buffer per promotion its body asked for (`text_return`: the
/// block tail's accumulator, a formatted early return's work text, a built local the tail
/// names), and any of them is a valid destination for a return — the call is leaving, so
/// nothing this frame does with the buffer afterwards matters — EXCEPT one the value
/// itself reads: writing `"{x}-{n}"` into `x` clears `x` before it is rendered.  Only a
/// HIDDEN buffer qualifies; a user-written `&text` parameter is the caller's variable, and
/// a return must not overwrite it.  `None` = the function has no buffer to deliver
/// through, which is the `__ret_N` residual.
pub(super) fn text_return_buffer_for(
    expr: &Value,
    function: &Function,
    data: &Data,
    d_nr: u32,
) -> Option<u16> {
    let mut read = HashSet::default();
    expr.walk(&mut |v| {
        if let Value::Var(x) = v {
            read.insert(*x);
        }
    });
    data.def(d_nr)
        .attributes()
        .iter()
        .filter(|a| {
            a.hidden && matches!(&a.typedef, Type::RefVar(t) if matches!(**t, Type::Text(_)))
        })
        .map(|a| function.var(&a.name))
        .find(|&v| v != u16::MAX && !read.contains(&v))
}

/// If `op` is a scope-exit free (`OpFreeRef` / `OpFreeText` /
/// `OpFreeRefIfDistinct`), return the var it frees.  The scopes-side twin of
/// `pre_eval::free_op_var` (generation is not depended on from here).
/// @PLN35 sub-class B — insert `frees` into every arm of an `If`/nested tail, just BEFORE
/// the arm's RESULT value (keeping the result as the tail). Used when a non-hoistable
/// `&text` return's arm allocates a sibling store: the frees run after the allocation
/// inside the arm instead of before the whole `return`. A store null on an arm's path
/// makes its `OpFreeRef` a no-op, so pushing into every arm is safe.
fn push_frees_into_arms(tail: &mut Value, frees: &[Value]) {
    match tail {
        Value::Span(b) => push_frees_into_arms(&mut b.1, frees),
        Value::If(_, then, els) => {
            push_frees_into_arms(then, frees);
            push_frees_into_arms(els, frees);
        }
        Value::Block(bl) => {
            let at = bl.operators.len().saturating_sub(1);
            for (i, fv) in frees.iter().enumerate() {
                bl.operators.insert(at + i, fv.clone());
            }
        }
        Value::Insert(ops) => {
            let at = ops.len().saturating_sub(1);
            for (i, fv) in frees.iter().enumerate() {
                ops.insert(at + i, fv.clone());
            }
        }
        leaf => {
            // A bare result value — wrap `[frees…, value]` in an `Insert` (a flat statement
            // sequence whose value is its last element).
            let v = std::mem::replace(leaf, Value::Null);
            let mut ops: Vec<Value> = frees.to_vec();
            ops.push(v);
            *leaf = Value::Insert(ops);
        }
    }
}

pub(super) fn scope_free_op_var(op: &Value, data: &Data) -> Option<u16> {
    if let Value::Call(d, args) = op.unspan()
        && data.op_sets().frees.contains(d)
        && let Some(arg0) = args.first()
        && let Value::Var(v) = arg0.unspan()
    {
        return Some(*v);
    }
    None
}

impl Scopes<'_> {
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn insert_free(
        &mut self,
        block: &Block,
        free: &[Value],
        is_return: bool,
        data: &Data,
        function: &mut Function,
    ) -> Vec<Value> {
        let mut res = Vec::new();
        let mut ls = Vec::new();
        let n = block.operators.len();
        // @PLN35 — the block's RESULT op is the last op that is NOT a scope-exit free.
        // A value-returning block can end in a free of a block-scoped local that was
        // materialised inside a branch (e.g. a `..rest` text read-temp, freed at the
        // common-parent block after the value-producing `if`): that trailing free is not
        // the block's value.  Hoisting it as the result minted `<int> __ret_N =
        // OpFreeText(local)` — an empty result on interp and invalid native (`= ;`).  So
        // when a value-result return-block's LAST op is a scope-free and its real result
        // is a plain value op (not a nested Block), treat that value op as the result and
        // run the trailing free(s) AFTER the hoist.  All other blocks keep `n-1`.
        let result_idx = if is_return
            && block.result != Type::Void
            && n > 0
            && scope_free_op_var(&block.operators[n - 1], data).is_some()
        {
            (0..n)
                .rev()
                .find(|&i| scope_free_op_var(&block.operators[i], data).is_none())
                .filter(|&i| !matches!(&block.operators[i], Value::Block(_)))
                .unwrap_or(n.wrapping_sub(1))
        } else {
            n.wrapping_sub(1)
        };
        let trailing_frees: Vec<Value> = if n > 0 && result_idx + 1 < n {
            block.operators[result_idx + 1..].to_vec()
        } else {
            Vec::new()
        };
        for (o_nr, o) in block.operators.iter().enumerate() {
            if o_nr > result_idx {
                // A trailing scope-free op (collected into `trailing_frees`); it runs
                // after the result hoist below, never as the block's value.
                continue;
            }
            if o_nr == result_idx {
                if let Value::Block(bl) = &block.operators[o_nr] {
                    for v in self.insert_free(bl, free, is_return, data, function) {
                        ls.push(v);
                    }
                } else if block.result == Type::Void || matches!(block.result, Type::Never) {
                    // `Never` joins `Void` here because it is the same SHAPE for free
                    // placement: a block that never completes yields no value, so there
                    // is nothing to hoist into a `__ret_N` and nothing to return — and
                    // the value leg below, having nothing to hoist, emitted the frees
                    // BEFORE the tail.  When that tail is a branch (a `match` whose arm
                    // `return`s is what types the block `never`), the arm that does NOT
                    // return then reads a variable already released: `null(oob)` on
                    // native, and on a droppable a drop before the arm plus a second one
                    // at the `return` — a use-after-free (loft#992).  The two legs below
                    // put the tail where it belongs either way: a tail that
                    // unconditionally returns keeps the frees in front of it, a tail that
                    // may still complete runs first and the frees follow.
                    //
                    // @P322 — when the function body ends with a nested
                    // Void-result block whose last op is `Return(...)` (the
                    // iterator-generator shape: `for n in […] { yield n; }
                    // return null;`), the OUTER-scope frees passed in via
                    // `free` must run BEFORE the return so function-scope
                    // owned locals (`__vdb_*` vector backings, etc.) get
                    // cleaned up.  Prior to this fix the void branch
                    // dropped `free` entirely and only emitted the inner
                    // `Return` + a redundant trailing `Return(Null)`, so
                    // function-scope vectors leaked at program exit.
                    let o_is_terminal = expr_ends_in_return(o);
                    if o_is_terminal {
                        for v in free {
                            ls.push(v.clone());
                        }
                        ls.push(o.clone());
                    } else {
                        ls.push(o.clone());
                        for v in free {
                            ls.push(v.clone());
                        }
                        ls.push(Value::Return(Box::new(Value::Null)));
                    }
                } else {
                    // @PLN85 poison-green — the B5-L3 invariant extended INTO
                    // block tails: return-site frees must not run before the tail
                    // expression EVALUATES.  The non-block leg (free_vars) has
                    // always hoisted a value tail into a `__ret_N` temp before the
                    // frees; this leg emitted `frees; Return(tail)` — the tail
                    // then evaluated AFTER the frees, and any store it still read
                    // (directly or through an alias no fact carries, e.g. a
                    // fn-ref read out of a struct field calling a closure record
                    // the host struct's free just cascaded) was a use-after-free:
                    // silent stale data without LOFT_POISON, deterministic
                    // garbage with it (the closure/field-capture family).
                    // A bare-Var tail is free-safe (its slot holds the value) —
                    // EXCEPT a `&τ` place ref (`RefVar`): returning it DEREFS the
                    // place DbRef at the Return, after the frees released the
                    // source store (the @PLN87 L3/L4 live-read shapes under
                    // poison).  Hoisting `Set(__ret_N, Var(r))` performs the
                    // deref NOW, before the frees.
                    let tail_needs_eval = match o.unspan() {
                        Value::Null => false,
                        // A `&τ` place ref derefs at the Return (hoist) —
                        // EXCEPT a `&text`: the promoted out-buffer returned
                        // per the text-return contract (alive in the caller;
                        // hoisting it broke native's buffer materialization).
                        Value::Var(v) => {
                            matches!(function.tp(*v), Type::RefVar(inner)
                            if !matches!(inner.base(), Type::Text(_)))
                            // A bare text LOCAL returned while the function holds a hidden
                            // buffer is delivered through that buffer and freed, like any
                            // other owned text return (@FR-F-Ret) — its slot does hold the
                            // value, but nothing frees the `String` behind it once the frame
                            // drops.  A lambda whose one buffer went to `tb` returned `ta`
                            // this way, one orphan per call (loft#1357).
                            || (is_return
                                && !function.is_argument(*v)
                                && !function.is_skip_free(*v)
                                && matches!(function.tp(*v).base(), Type::Text(_))
                                && !matches!(function.tp(*v), Type::RefVar(_))
                                && any_text_return_buffer(function, data, self.d_nr)
                                    .is_some_and(|b| b != *v))
                        }
                        _ => true,
                    };
                    // Text results take the same hoist with the text-leg
                    // mechanics: the temp's String owns a byte copy, and
                    // `skip_free` keeps its OpFreeText out of the scope exit
                    // (the caller copies bytes immediately on return — the
                    // established `__ret_N` text contract pre_eval/native read).
                    let is_text_result = matches!(block.result.base(), Type::Text(_));
                    // @PLN35 sub-class A — a heap-record / vector return (a `Reference`
                    // struct, a struct-`Enum(_, true)`, or a `Vector`) ALSO takes the hoist:
                    // when the block's tail is an `if`/`match` whose taken arm ALLOCATES a
                    // sibling store (a `..rest` materialisation's `__vdb`), the un-hoisted
                    // `frees; return <if>` emits `OpFreeRef(__vdb)` BEFORE the allocation
                    // inside the return → the store leaks (`ANALYSIS.md`, oracle
                    // FREE-before-ALLOC). Hoisting to a `__ret` temp runs the allocation
                    // first. A DbRef `__ret` is native-safe (the `&text` out-buffer is NOT —
                    // it is excluded above via `tail_needs_eval`).
                    let is_heap_ref_result = matches!(
                        block.result.base(),
                        Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
                    );
                    // loft#1469 — the fn-ref block result, the fourth member of the same
                    // list.  A `match` in tail position lowers to a `scalar_match` BLOCK
                    // whose last op is a `Return`, so it reaches this leg rather than
                    // `free_vars`'s; without the type here the tail stayed un-hoisted, the
                    // two arms of one choice pushed different WIDTHS — 20 bytes for the
                    // capturing arm, 8 for the bare def-number — and the return read twelve
                    // bytes of uninitialised stack as the closure half.  The `if` spelling of
                    // the identical choice is correct precisely because it IS hoisted, which
                    // gives both arms a `fn`-typed destination to be padded against.
                    let is_fnref_result = matches!(block.result.base(), Type::Function(..));
                    let mut hoist_tmp: Option<u16> = None;
                    if is_return
                        && (!free.is_empty() || !trailing_frees.is_empty())
                        && (is_value_return_type(&block.result)
                            || is_text_result
                            || is_heap_ref_result
                            || is_fnref_result)
                        && tail_needs_eval
                        && !expr_ends_in_return(o)
                    {
                        if is_text_result
                            && !matches!(o.unspan(), Value::Null)
                            && let Some(buf) = text_return_buffer_for(o, function, data, self.d_nr)
                        {
                            // @FR-F-Ret / @FR-F-Call — the block-tail twin of `free_vars`'s
                            // text delivery: an owned text return goes into the CALLER's
                            // hidden `&text` buffer, never into a frame-local temp nothing
                            // frees.  This is the leg an early `return a ?? b` reaches (its
                            // `??` lowers to a block whose tail is the `if`), and it orphaned
                            // one String per call on the interpreter (loft#1338).  Per arm,
                            // so native's arm types stay uniform; then the temps the copy
                            // drained are freed, the scope frees follow, and the buffer is
                            // what the `Return` below names.
                            let mut delivered = o.clone();
                            crate::parser::Parser::push_text_arms_into(
                                &mut delivered,
                                buf,
                                data.def_nr("OpCreateStack"),
                            );
                            ls.push(delivered);
                            let pending: Vec<Value> =
                                trailing_frees.iter().chain(free.iter()).cloned().collect();
                            free_copied_text_sources(&mut ls, o, &pending, function, data);
                            hoist_tmp = Some(buf);
                        } else {
                            self.ret_temp_counter += 1;
                            let name = format!("__ret_{}", self.ret_temp_counter);
                            let tmp = function.add_temp_var(&name, &block.result);
                            // The hoisted value is the RETURN value (transferred to the
                            // caller): its scope-exit free must NOT fire, else the caller
                            // reads a freed record.  Text already does this — and for text it
                            // is the ORPHAN `free_vars`'s residual arm documents: kept only
                            // where the function has no buffer to deliver through.  A heap
                            // ref/vector needs it too.
                            if is_text_result || is_heap_ref_result {
                                function.set_skip_free(tmp);
                            }
                            self.var_scope.insert(tmp, self.scope);
                            self.var_order.push(tmp);
                            ls.push(v_set(tmp, o.clone()));
                            if is_text_result {
                                // The copy drained the `??` temp inside the tail; free it
                                // even where the temp itself is the residual orphan.
                                let pending: Vec<Value> =
                                    trailing_frees.iter().chain(free.iter()).cloned().collect();
                                free_copied_text_sources(&mut ls, o, &pending, function, data);
                            }
                            hoist_tmp = Some(tmp);
                            // A buffer the tail READS is still the delivery once the value is
                            // staged (the `free_vars` twin says why): move the temp's bytes
                            // into it, free the temp, and return the buffer (loft#1357).
                            if is_text_result
                                && !matches!(o.unspan(), Value::Null)
                                && let Some(buf) = any_text_return_buffer(function, data, self.d_nr)
                            {
                                ls.push(v_set(buf, Value::Var(tmp)));
                                ls.push(call("OpFreeText", tmp, data));
                                hoist_tmp = Some(buf);
                            }
                        }
                    }
                    // The block's OWN trailing scope-frees (after the result op) run first,
                    // then the enclosing scope's `free`; both after the result is hoisted.
                    let mut ret_frees: Vec<Value> = Vec::new();
                    for v in trailing_frees.iter().chain(free.iter()) {
                        // A free of a var the RETURNED tail expression still READS
                        // cannot run before it — that is a use-after-free (the
                        // `?? [literal]` return-tail class: interp read the freed
                        // store silently — LOFT_POISON turns it into a SIGSEGV —
                        // and native crashed on the 65535 sentinel).  Its freeing
                        // is owned INSIDE the expression instead: the return-
                        // delivery materializer consumes an owned-fresh arm local
                        // after its append, on EVERY path (cross-arm frees).  So
                        // the pre-return free is DROPPED here, not moved (even
                        // under the hoist — the materializer already freed the
                        // consumed store inside the expression; re-emitting the
                        // free after the temp would double-free it).
                        if is_return
                            && let Some(fv) = scope_free_op_var(v, data)
                            && o.reads_var(fv)
                            && function.is_arm_consumed(fv)
                        {
                            continue;
                        }
                        ret_frees.push(v.clone());
                    }
                    // @PLN35 sub-class B — a NON-hoistable `&text` (RefVar-text) tail whose `If`
                    // arm ALLOCATES a sibling store: the frees can't run before the return (they
                    // would precede the arm's `OpDatabase` → the store leaks) and the `&text`
                    // out-buffer can't hoist (native `Str::new(&local)` dangle, excluded via
                    // `tail_needs_eval`). Push the frees INTO each arm, just before the arm's
                    // result, so they run AFTER the allocation and the buffer is yielded raw.
                    let is_refvar_text = matches!(block.result.base(),
                        Type::RefVar(inner) if matches!(inner.base(), Type::Text(_)));
                    if hoist_tmp.is_none()
                        && is_return
                        && is_refvar_text
                        && !ret_frees.is_empty()
                        && matches!(o.unspan(), Value::If(_, _, _))
                    {
                        let mut tail = o.clone();
                        push_frees_into_arms(&mut tail, &ret_frees);
                        ls.push(Value::Return(Box::new(tail)));
                    } else {
                        ls.extend(ret_frees);
                        if let Some(tmp) = hoist_tmp {
                            ls.push(Value::Return(Box::new(Value::Var(tmp))));
                        } else if is_return {
                            ls.push(Value::Return(Box::new(o.clone())));
                        } else {
                            ls.push(o.clone());
                        }
                    }
                }
            } else {
                ls.push(o.clone());
            }
        }
        res.push(Value::Block(Box::new(Block {
            name: block.name,
            operators: ls,
            result: block.result.clone(),
            scope: block.scope,
            var_size: 0,
        })));
        res
    }
}

/// True when `expr` is a `Return` (or recursively ends with one through
/// `Insert`/`Block` wrappers).  Used by `free_vars` to decide whether the
/// B5-L3 `__ret_N` wrap is safe — wrapping a terminal expression would
/// produce `let _ret = return …` in native and double-emit the inner
/// Return inside the Set's expression generator.
pub(super) fn expr_ends_in_return(expr: &Value) -> bool {
    matches!(expr.tail(), Value::Return(_))
}
