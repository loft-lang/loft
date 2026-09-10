// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Pre-evaluation pass: hoist complex subexpressions into temp variables
//! before emitting the main code.  This avoids nested borrow conflicts in
//! generated Rust code where `stores` would be borrowed mutably twice.

use crate::data::{Block, Type, Value};
use crate::data_store::ValueType;
use crate::ir_node::IrNode;
use std::collections::{HashMap, HashSet};

use super::{Output, PreEvalEntry, narrow_int_cast};

/// Is `code` a STATEMENT SEQUENCE — Rust text whose value is its tail expression,
/// preceded by at least one statement — rather than a single expression?
///
/// A pre-eval binding is emitted as `let _pre_N = <code>;`, which is an expression
/// position, so text like `let mut var_x: DbRef = DbRef::NULL; if c { … } else { … }`
/// is a syntax error there ("expected expression, found `let` statement").  Two
/// separate lowerings produce that prefix — an `if` whose test lifted a call, and an
/// `if` whose branch variables must be declared before the arms — and the wrap that
/// made the first one an expression was written into that lowering, so the second one
/// never got it (loft#910 follow-up).  Asking the TEXT keeps the two from drifting
/// again: whatever the producer, what lands in a `let` binding must be an expression.
///
/// A `;` inside braces, parens, brackets, a string or char literal, or a comment does
/// not count — only one at depth 0, which is what separates statements.
fn is_statement_sequence(code: &str) -> bool {
    let b = code.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth -= 1,
            // A `;` at depth 0 ends a statement — unless it is the very last
            // character, where it is just a trailing terminator.
            b';' if depth <= 0 => {
                if code[i + 1..].trim().is_empty() {
                    return false;
                }
                return true;
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            // `'` is a char literal (`'x'`, `'\n'`) or a lifetime (`'static`).  Only
            // the literal has a closing quote to skip to; a lifetime is ordinary text.
            b'\'' => {
                let esc = b.get(i + 1) == Some(&b'\\');
                let close = if esc { i + 3 } else { i + 2 };
                if b.get(close) == Some(&b'\'') {
                    i = close;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Intrinsic identity of an IR node: its address in the (frozen) IR tree.
///
/// Both pre-eval walks — `collect_pre_evals` then `output_code_with_subst` —
/// traverse the *same* `&Value` without mutating it, so any sub-node has the
/// same address in each walk.  Identity is therefore a property of the node,
/// not of the walk.  This is the fix for the recurring counter-coupling class
/// (see COMPILER.md § Synthesised-identity stability): the old recogniser
/// re-derived identity by regenerating the node's Rust text at a rewound
/// `counter` and string-comparing — which silently fails when the regenerated
/// text drifts (e.g. an `Op*` operand whose inner `_pre_N` names differ), so
/// the node is re-inlined and any side effect is evaluated twice (#272).
#[inline]
fn node_id(v: &Value) -> usize {
    std::ptr::from_ref(v) as usize
}

/// Explicit shared state between the collect walk and the emit walk: which IR
/// nodes were hoisted into a `let _pre_N = …;` binding, and under what name.
///
/// Built once by `collect_pre_evals`; the emit walk then reads it through
/// `name_map` (installed as `Output::active_pre_eval`).  `by_node` is the
/// authoritative, intrinsic lookup — node identity (address) → entry — so a
/// hoisted node emits its `_pre_N` name with no counter re-derivation and no
/// regenerate-and-string-match.  This is the single source of truth for
/// pre-eval identity (see COMPILER.md § Synthesised-identity stability).
#[derive(Default)]
pub(super) struct PreEvalSet {
    /// Hoisted bindings in collection (= emission) order.
    pub entries: Vec<PreEvalEntry>,
    /// Intrinsic node identity → index into `entries`.
    by_node: HashMap<usize, usize>,
}

impl PreEvalSet {
    /// Register one hoisted binding, keyed on the source node's intrinsic identity.
    fn push(&mut self, node: &Value, entry: PreEvalEntry) {
        let idx = self.entries.len();
        self.by_node.insert(node_id(node), idx);
        self.entries.push(entry);
    }

    /// The address→name map the emit walk consults at every node
    /// (`Output::active_pre_eval`).  This is the single source of truth for
    /// pre-eval identity during emission: a node whose address is a key emits
    /// its `_pre_N` name instead of being regenerated inline.
    pub(super) fn name_map(&self) -> HashMap<usize, String> {
        self.by_node
            .iter()
            .map(|(&addr, &i)| (addr, self.entries[i].0.clone()))
            .collect()
    }
}

/// True if `v` reads any variable in `vars`, at any depth.  Used by @P312 to
/// detect a call argument that reads a variable which ANOTHER argument passes
/// as a fresh `&mut var_X` borrow — that read must be hoisted before the call
/// or native emits `f(&mut var_x, g(var_x))` and the Copy-read of `var_x` while
/// it is mutably borrowed trips rustc E0503.  Mirrors `contains_op_database`'s
/// traversal but also descends `Span`/`Drop`/`Return`/`Iter` and tests `Var` /
/// `CallRef` leaves.
fn value_refs_any(node: IrNode, vars: &HashSet<u16>) -> bool {
    match node.kind() {
        ValueType::Var => vars.contains(&node.var_nr()),
        ValueType::CallRef => {
            vars.contains(&node.callref_var())
                || node
                    .callref_args()
                    .iter()
                    .any(|arg| value_refs_any(arg, vars))
        }
        ValueType::Call => node.call_args().iter().any(|arg| value_refs_any(arg, vars)),
        ValueType::Insert => node
            .insert_items()
            .iter()
            .any(|arg| value_refs_any(arg, vars)),
        ValueType::Block => node
            .as_block()
            .operators()
            .iter()
            .any(|op| value_refs_any(op, vars)),
        ValueType::Set => vars.contains(&node.set_var()) || value_refs_any(node.set_inner(), vars),
        ValueType::If => {
            value_refs_any(node.if_cond(), vars)
                || value_refs_any(node.if_then(), vars)
                || value_refs_any(node.if_else(), vars)
        }
        ValueType::Drop => value_refs_any(node.drop_inner(), vars),
        ValueType::Return => value_refs_any(node.return_inner(), vars),
        ValueType::Iter => {
            vars.contains(&node.iter_var())
                || value_refs_any(node.iter_create(), vars)
                || value_refs_any(node.iter_next(), vars)
                || value_refs_any(node.iter_init(), vars)
        }
        ValueType::Span => value_refs_any(node.span_inner(), vars),
        _ => false,
    }
}

impl Output<'_> {
    /// Use this instead of emitting an argument block when the block exists only to pass a
    /// local text variable by mutable reference. Returns the variable index so the call site
    /// can emit `&mut var_<name>` without generating a spurious empty block expression.
    pub(super) fn create_stack_var(&self, v: &Value) -> Option<u16> {
        // `Value::unspan`'s rule: a site that pattern-matches a specific variant peels first.
        // Both tests below are BINDINGS with no catch-all to fall into, so a wrapper does not
        // pick a different arm — it answers `None`, and the `&mut var_…` a by-ref argument
        // needs is then never emitted, with nothing to report it.
        let v = v.unspan();
        // Direct OpCreateStack call on a variable (text or numeric by-ref): `fn f(x: &T)` called as `f(v)`.
        // The parser wraps the argument as Value::Call("OpCreateStack", [Value::Var(n)]).
        // output_call emits nothing for OpCreateStack, so we must intercept here and emit
        // `&mut var_<name>` instead.
        if let Value::Call(d_nr, args) = v
            && self.data.def(*d_nr).name() == "OpCreateStack"
            && let [Value::Var(nr)] = args.as_slice()
        {
            return Some(*nr);
        }
        let Value::Block(bl) = v else { return None };
        // Handle DbRef-stack refs: Type::Reference with OpCreateStack ops.
        if let Type::Reference(_, vars) = &bl.result {
            let [vr] = vars.as_slice() else { return None };
            let only_create_stack = bl
                .operators
                .iter()
                .filter(|op| !matches!(op, Value::Line(_)))
                .all(|op| matches!(op, Value::Call(d_nr, _) if self.data.def(*d_nr).name() == "OpCreateStack"));
            return only_create_stack.then_some(*vr);
        }
        None
    }

    /// @P312 — the set of variables that some argument passes as a fresh
    /// `&mut var_X` borrow (`create_stack_var`).  A by-value/Copy read of such a
    /// var in another argument trips rustc E0503 ("cannot use `var_X` because it
    /// was mutably borrowed"), so those reads are hoisted into `_pre_N` bindings
    /// before the call.  The `&`-param FORWARDING case (passing an existing
    /// `&mut` ref onward, emitted as bare `var_X`) is intentionally excluded:
    /// Rust's two-phase reborrow tolerates a read alongside it, so hoisting it
    /// would only bloat the generated code.
    fn borrowed_arg_vars(&self, vals: &[Value]) -> HashSet<u16> {
        let mut set = HashSet::new();
        for arg in vals {
            if let Some(vr) = self.create_stack_var(arg) {
                set.insert(vr);
            }
        }
        set
    }

    /// Fix the "hoisted return value" pattern inserted by `scopes::free_vars`.
    ///
    /// When a function returns early (`return expr`) and has local text/ref variables
    /// that need cleanup, `scopes::free_vars` transforms the return into:
    ///   `[expr, OpFreeText(v)…, Return(Null)]`
    /// so the interpreter can push `expr` onto the stack before freeing locals and returning.
    ///
    /// In native Rust code, `OpFreeText` is a no-op (Rust drops automatically), so the
    /// pattern degenerates to `expr; return ();` which drops the return value and fails to
    /// compile when the function return type is not void.
    ///
    /// This method detects the pattern in a slice of block operators and returns a patched
    /// copy where `Return(Null)` is replaced by `Return(expr)` and `expr` is removed from
    /// its earlier position.
    pub(super) fn patch_hoisted_returns<'a>(
        &self,
        ops: &'a [Value],
    ) -> std::borrow::Cow<'a, [Value]> {
        let fn_returned = self.data.def(self.def_nr).returned();
        if matches!(fn_returned, Type::Void) {
            return std::borrow::Cow::Borrowed(ops);
        }
        // Pass-3 dedupe: ONE free-op recognizer (`free_op_var`) — this
        // closure and `freed_var` below were stale hand-rolled copies that
        // missed `OpFreeRefIfDistinct` (the #330 alias-witness free), so a
        // window containing one mis-classified the op.
        let is_free_op = |op: &Value| Self::free_op_var(op, self.data).is_some();

        // Pass 1 — B5-L3 text-temp collapse:
        //   [Set(__ret_N, Call(...)), ..., Return(Var(__ret_N))]
        //     → [..., Return(Call(...))]
        //
        // The Set+Return dance is emitted by scopes.rs::free_vars to keep
        // the interpreter happy (it copies bytes into __ret_N so subsequent
        // OpFreeText on the original work buffer doesn't dangle the TOS
        // Str).  Native would materialise Set(__ret_N, Call) as
        // `let var___ret: String = call(...).to_string()` and Return(Var)
        // as `return Str::new(&var___ret)` — the local String drops at
        // function exit and the returned Str's raw ptr dangles
        // (`tests/scripts/86-interfaces.loft::if_label`).  For this narrow
        // pattern, the inner Call already returns a borrow into a program-
        // lifetime Store; collapsing to `return Call(...)` keeps that
        // borrow intact.
        //
        // Safety criteria — all must hold:
        //   • Set target var name starts with `__ret_` and is skip_free.
        //   • Target type is Type::Text(_).
        //   • Set value is a `Value::Call` (not If/Match/Block — those can
        //     borrow from free-op targets and would dangle post-collapse).
        //   • Return is `Return(Var(target))`.
        //   • No other operator between Set and Return reads or writes
        //     the target var.
        let variables = self.data.def(self.def_nr).variables();
        let mut result: Vec<Value> = ops.to_vec();
        let mut ret_search_from = 0;
        while let Some(ret_pos) = result[ret_search_from..]
            .iter()
            .position(|op| {
                matches!(op.unspan(), Value::Return(v) if matches!(v.as_ref().unspan(), Value::Var(target) if
                    variables.name(*target).starts_with("__ret_")
                    && variables.is_skip_free(*target)
                    // @PLN25/@PLN85: peel `Optional` — a `-> text?` fn's null-path
                    // ret-temp is `optional(text)` and rides the SAME B5-L3 collapse;
                    // unmatched, the local `String` temp survives to `return
                    // Str::new(&local)` and the caller memcpy's a dangling/null ptr
                    // (ptr::copy_nonoverlapping UB — the D-own-1 text-corpus t4 cell).
                    && matches!(variables.tp(*target).base(), Type::Text(_))))
            })
            .map(|p| p + ret_search_from)
        {
            ret_search_from = ret_pos + 1;
            // Extract the Return's target var_nr.
            let target = if let Value::Return(inner) = result[ret_pos].unspan()
                && let Value::Var(v) = inner.as_ref().unspan()
            {
                *v
            } else {
                continue;
            };
            // Find the preceding Set(target, <safe-to-inline>).
            // A value is safe to inline into the Return position when it
            // doesn't borrow from any local that might be freed between
            // Set and Return.  Calls to user fns / string literals / vars
            // (args or program-lifetime) qualify; If/Match/Block can
            // borrow from __work_N locals that free ops clobber, so
            // they're excluded.
            let set_pos = result[..ret_pos].iter().position(|op| {
                matches!(op.unspan(), Value::Set(v, val) if *v == target
                && matches!(
                    val.as_ref().unspan(),
                    Value::Call(_, _) | Value::Text(_)
                ))
            });
            let Some(set_idx) = set_pos else { continue };
            // No other use of `target` between set_idx+1 and ret_pos-1.
            let target_used_between = result[set_idx + 1..ret_pos]
                .iter()
                .any(|op| op.reads_var(target));
            if target_used_between {
                continue;
            }
            // @P364: the collapse HOISTS the Set's Call from `set_idx` (which
            // is BEFORE the intervening free-ops) to `ret_pos` (AFTER them).
            // If that Call borrows a local that one of those free-ops frees,
            // the hoist is a use-after-free.  Concretely:
            //   __ret = jv.field(v).as_text();  OpFreeRef(v);  return __ret
            // must NOT become  OpFreeRef(v);  return field(v)…  — the field
            // call would then read `v` after it was freed + nulled
            // (store_nr = u16::MAX → `allocation.rs` index-out-of-bounds).
            // The comment above assumes the inner Call borrows only a
            // program-lifetime Store; when it borrows a scope-freed local
            // that assumption breaks, so skip the collapse and keep the
            // Set+Return form (Call stays before the free, copy is returned).
            let set_call_borrows_freed_var = {
                let call_val = match result[set_idx].unspan() {
                    Value::Set(_, inner) => Some(inner.as_ref()),
                    _ => None,
                };
                call_val.is_some_and(|cv| {
                    let crosses_a_free = result[set_idx + 1..ret_pos]
                        .iter()
                        .any(|op| Self::free_op_var(op, self.data).is_some());
                    crosses_a_free && Self::reads_a_local(cv, variables)
                })
            };
            if set_call_borrows_freed_var {
                continue;
            }
            // Perform the collapse: extract the Call from the Set,
            // remove the Set, and rewrite Return(Var) → Return(Call).
            // Use unspan_mut so a Span-wrapped Set still yields the
            // inner value when destructured.
            let removed = result.remove(set_idx);
            let inner = match removed {
                Value::Span(b) => b.1,
                other => other,
            };
            let Value::Set(_, call_box) = inner else {
                unreachable!("set_pos pointed at a Set");
            };
            let ret_pos_after = ret_pos - 1;
            result[ret_pos_after] = Value::Return(call_box);
            ret_search_from = ret_pos_after + 1;
        }

        // Pass 2 — existing Return(Null) expr hoist.
        // Quick check: is there any Return(Null) left?
        let has_return_null = result
            .iter()
            .any(|op| matches!(op.unspan(), Value::Return(v) if *v.unspan() == Value::Null));
        if !has_return_null {
            return if result.len() == ops.len() && result.iter().zip(ops).all(|(a, b)| a == b) {
                std::borrow::Cow::Borrowed(ops)
            } else {
                std::borrow::Cow::Owned(result)
            };
        }
        // The @P274 use-after-free guard MUST see every free flavour —
        // the old hand-rolled copy here missed `OpFreeRefIfDistinct`, so a
        // hoist past an if-distinct free of one of the expr's operands went
        // undetected.  `free_op_var` is the one recognizer.
        let freed_var = |op: &Value| -> Option<u16> { Self::free_op_var(op, self.data) };
        let mut search_from = 0;
        while let Some(ret_pos) = result[search_from..]
            .iter()
            .position(|op| matches!(op.unspan(), Value::Return(v) if *v.unspan() == Value::Null))
            .map(|p| p + search_from)
        {
            // Find the nearest preceding expression that is not a free-op, Line, or Return.
            let expr_pos = result[..ret_pos]
                .iter()
                .rposition(|op| !matches!(op.unspan(), Value::Line(_)) && !is_free_op(op.unspan()));
            if let Some(idx) = expr_pos {
                // @P274 guard: refuse the hoist when any intervening free-op
                // between `idx` and `ret_pos` frees a variable that `expr`
                // references.  Hoisting `expr` into `Return(expr)` would
                // place the use AFTER the free in the emitted Rust, since
                // free-ops keep their original position while the expression
                // moves into the tail return — classic use-after-free
                // (`stores[var.store_nr]` panics with `index out of bounds:
                // the len is N but the index is 65535` because OpFreeRef
                // sets the freed var's store_nr to u16::MAX before the
                // hoisted expr can use it).  Leaving the original
                // `[expr, free, Return(Null)]` shape intact lets
                // `detect_ref_tail_capture` (output_block) emit the
                // `let __native_tail_ret = expr; free; return __native_tail_ret;`
                // pattern, which orders the use BEFORE the free correctly.
                let expr = &result[idx];
                // A void control-flow value — an `if` with a `Null` else (whose
                // taken branch diverges via `return`), a Set/Drop, a void call —
                // is a STATEMENT, not the block's return value. Hoisting it into
                // `Return(expr)` produces `Str::new(())` / `().to_string()`
                // (E0308/E0599); this is exactly the enum-dispatch shape
                // `[if tag==a {return …}, if tag==b {return …}, return null]`.
                // Leave it as a statement and keep the typed-null `Return(Null)`.
                if self.is_void_value(expr.unspan()) {
                    search_from = ret_pos + 1;
                    continue;
                }
                let mut conflict = false;
                for between in &result[idx + 1..ret_pos] {
                    if let Some(v) = freed_var(between)
                        && expr.reads_var(v)
                    {
                        conflict = true;
                        break;
                    }
                }
                if conflict {
                    search_from = ret_pos + 1;
                    continue;
                }
                let expr = result.remove(idx);
                // ret_pos shifted by -1 because we removed one element before it.
                let actual_ret = ret_pos - 1;
                result[actual_ret] = Value::Return(Box::new(expr));
                search_from = actual_ret + 1;
            } else {
                search_from = ret_pos + 1;
            }
        }
        std::borrow::Cow::Owned(result)
    }

    /// Does `val` read any LOCAL of this function — as opposed to an argument, a literal, or a
    /// call whose result borrows a program-lifetime store?
    ///
    /// This is the safety question for hoisting a value past a scope-exit free, and it is asked
    /// this way round because the other way round cannot be answered here.  Naming the vars a
    /// free INVALIDATES needs the whole ownership graph: `OpFreeRef(__vdb_1)` takes the store
    /// that `tv: vector<text>` is a VIEW into (`tv`'s type carries `__vdb_1` as a dep, and the
    /// expression never names `__vdb_1`), while `OpFreeRef(___clos_1)` CASCADES through the
    /// closure record to the `__cell_text` a boxed capture lives in — a runtime ownership fact
    /// with no dep to read. Asking only whether the expression reads the FREED VAR answered
    /// "no" for both, and `return tv[0]` was hoisted past the free of its own container
    /// (loft#1235).
    ///
    /// A local is therefore treated as reachable from any free, which over-approximates: the
    /// cost of a false positive is that the `Set` + `Return` pair is KEPT, which returns an
    /// owned copy made before the free — correct, and one `to_string` more than the collapse.
    /// The collapse still fires for the shape it was written for, a call over ARGUMENTS whose
    /// result borrows a program-lifetime store (`fn if_label<T: Labelable>(x: T) -> text {
    /// x.to_label() }`), and for any window with no free in it at all.
    fn reads_a_local(val: &Value, variables: &crate::variables::Function) -> bool {
        (0..variables.count()).any(|v| !variables.is_argument(v) && val.reads_var(v))
    }

    /// @P364: if `op` is a scope-exit free (`OpFreeRef` / `OpFreeText` /
    /// `OpFreeRefIfDistinct`), return the var it frees (its first `Var`
    /// argument); otherwise `None`.  Used by the B5-L3 text-temp collapse
    /// to avoid hoisting a Call past a free of one of the Call's operands.
    pub(crate) fn free_op_var(op: &Value, data: &crate::data::Data) -> Option<u16> {
        if let Value::Call(d, args) = op.unspan()
            && data.op_sets().frees.contains(d)
            && let Some(arg0) = args.first()
            && let Value::Var(v) = arg0.unspan()
        {
            return Some(*v);
        }
        None
    }

    /// Use this to detect sub-expressions that would cause a double-borrow of `stores`
    /// if left inline and must therefore be hoisted into `let _preN` bindings.
    /// Returns true if the named native Op function uses `stores` in its special-case emit code.
    /// These functions need pre-eval treatment to avoid double-borrow of `stores` when they
    /// appear as arguments inside other stores-using calls.
    fn op_uses_stores(name: &str) -> bool {
        matches!(
            name,
            "OpNewRecord"
                | "OpFinishRecord"
                | "OpLinkRecord"
                | "OpGetRecord"
                | "OpIterate"
                | "OpDatabase"
                | "OpCopyRecord"
                | "OpSizeofRef"
                | "OpStep"
                | "OpRemove"
                | "OpHashRemove"
                | "OpAppendCopy"
                | "OpFormatDatabase"
                | "OpFormatStackDatabase"
                // @PLN25 E2 — vector-element reads emit `stores.vec_get/ref_or_raise`
                // calls, so when nested as an arg to another stores-using op (e.g. a
                // `__nullable<S>` element read fed to a `stores.*` template) they must be
                // hoisted into a local first — else `stores.f(&(stores.vec_get(…)))` is a
                // double mutable borrow (rustc E0499).
                | "OpGetVector"
                | "OpGetVectorNullable"
                | "OpVectorRef"
                | "OpVectorRefNullable"
        )
    }

    fn needs_pre_eval(&self, v: &Value) -> bool {
        // `Value::unspan`'s doc makes this an obligation, and here it is load-bearing rather
        // than tidy: the arms below single out `Call` / `Block` / `CallRef` / `Insert` /
        // `Iter`, all of which can answer TRUE, while a `Span` wrapper matches none of them
        // and takes `_ => false`.  So a spanned call was reported as needing no
        // pre-evaluation, which is exactly the double-borrow this analysis exists to prevent.
        // Measured over 45 corpus programs on `--native`: 1807 spanned values arrive here and
        // 1264 of them change answer once unspanned.
        let v = v.unspan();
        match v {
            Value::Call(d_nr, vals) => {
                let def = self.data.def(*d_nr);
                // User-defined functions (rust template is empty AND have loft code body)
                // always need pre-eval to avoid double-borrow.
                if def.rust().is_empty() && *def.code() != Value::Null {
                    true
                } else if def.rust().contains("stores")
                    // A `#rust` template written in `s.database.` vocabulary
                    // rewrites to `stores.` in native code (calls.rs), so it
                    // double-borrows when nested too — e.g. `OpGetVectorNullable`
                    // (`…&s.database.allocations`) nested as an arg to another
                    // `stores.*` template.
                    || def.rust().contains("s.database.")
                {
                    // Template fns that use `stores` can cause double-borrow when nested
                    // inside another stores-using call; treat them as needing pre-eval.
                    true
                } else if Self::op_uses_stores(def.name()) {
                    // Native Op functions whose special-case emit code passes `stores`
                    // also cause double-borrow when nested inside other stores-using calls.
                    // (Not gated on an empty rust template: some such ops — the vector
                    // reads — carry a template yet still emit a `stores.*` special case.)
                    true
                } else if def.rust().is_empty()
                    && *def.code() == Value::Null
                    && !def.name().starts_with("Op")
                {
                    // User-fn stubs (no rust template, no loft body, not a built-in Op)
                    // are emitted as todo!() but still take `&mut Stores` — pre-eval
                    // them to avoid double-borrow when they appear as nested arguments.
                    true
                } else {
                    vals.iter().any(|a| self.needs_pre_eval(a))
                }
            }
            // CallRef dispatches via match to user functions that take &mut Stores.
            // Block, Insert, and Iter contain statements that use stores.
            Value::Block(_) | Value::CallRef(_, _) | Value::Insert(_) | Value::Iter(..) => true,
            Value::If(test, t, f) => {
                self.needs_pre_eval(test) || self.needs_pre_eval(t) || self.needs_pre_eval(f)
            }
            Value::Drop(v) => self.needs_pre_eval(v),
            _ => false,
        }
    }

    /// Use this when you need the generated text of an expression for substitution or comparison,
    /// rather than writing it directly to the output stream.
    pub(super) fn generate_expr_buf(&mut self, v: &Value) -> std::io::Result<String> {
        let mut buf = std::io::BufWriter::new(Vec::new());
        self.output_code_inner(&mut buf, v)?;
        Ok(String::from_utf8(buf.into_inner()?).unwrap())
    }

    /// Use this to identify all sub-expressions in `v` that must be hoisted before the enclosing
    /// expression to prevent simultaneous `&mut Stores` borrows.
    /// Returns `(var_name, expr_code)` pairs ordered innermost-first so each pre-eval
    /// can safely reference earlier ones.
    pub(super) fn collect_pre_evals(&mut self, v: &Value) -> std::io::Result<PreEvalSet> {
        let mut result = PreEvalSet::default();
        self.collect_pre_evals_inner(v, &mut result)?;
        Ok(result)
    }

    /// Use this as the recursive worker for `collect_pre_evals`.
    /// Splitting from the wrapper keeps the result allocated once, and the pre-eval
    ///  counter is globally unique within a block.
    fn collect_pre_evals_inner(
        &mut self,
        v: &Value,
        result: &mut PreEvalSet,
    ) -> std::io::Result<()> {
        // Recurse into wrapper nodes so nested Call nodes inside Set/Drop/If are found.
        // @P312 — a bare user-fn call STATEMENT is wrapped in `Span` (source-position
        // metadata); without unwrapping it here the call's args are never analysed, so
        // the ref-alias hoist below never fires.  `Span` is transparent for codegen.
        if let Value::Span(s) = v {
            return self.collect_pre_evals_inner(&s.1, result);
        }
        if let Value::Set(_, rhs) = v {
            return self.collect_pre_evals_inner(rhs, result);
        }
        if let Value::Drop(inner) | Value::Return(inner) = v {
            return self.collect_pre_evals_inner(inner, result);
        }
        if let Value::If(test, true_v, false_v) = v {
            self.collect_pre_evals_inner(test, result)?;
            self.collect_pre_evals_inner(true_v, result)?;
            return self.collect_pre_evals_inner(false_v, result);
        }
        // CallRef dispatches to user functions — same hoisting rules as user-defined Call.
        // The closure arg appears once per candidate match arm (all arms receive the same
        // allocation block), so use replace_all=true to substitute every occurrence.
        if let Value::CallRef(_, args) = v {
            // @P312 — same ref-alias hoist as the user-fn Call arm below.
            let borrowed = self.borrowed_arg_vars(args);
            for arg in args {
                let needs_pre = self.create_stack_var(arg).is_none()
                    && (Self::is_sequence_arg(arg)
                        || self.needs_pre_eval(arg)
                        || (!borrowed.is_empty()
                            && value_refs_any(IrNode::Native(arg), &borrowed)));
                if needs_pre {
                    let name = format!("_pre_{}", self.counter);
                    self.counter += 1;
                    self.rewrite_code(result, arg, name, true)?;
                } else {
                    self.collect_pre_evals_inner(arg, result)?;
                }
            }
            return Ok(());
        }
        if let Value::Call(d_nr, vals) = v {
            // loft#885 stage 2 — a scalar read of a hoisted vector element emits as ONE
            // call, so its inner `OpGetVector*` must NOT be hoisted into a `let _pre_N`:
            // the fused emission ignores that binding, and the read would then happen
            // twice.  Both sides ask `fused_element_read`, so they cannot disagree about
            // which shape is fused.  Only the index and the field can still hold work.
            if let Some(fused) = self.fused_element_read(self.data.def(*d_nr).name(), vals) {
                self.collect_pre_evals_inner(fused.index, result)?;
                return self.collect_pre_evals_inner(fused.fld, result);
            }
            // @PLN157 P4b — the write twin: a fused element WRITE also folds its inner
            // element address away, so that address must not be lifted either.  Both
            // sides ask `fused_element_write`, so they cannot disagree.
            if let Some(fused) = self.fused_element_write(self.data.def(*d_nr).name(), vals) {
                self.collect_pre_evals_inner(fused.index, result)?;
                self.collect_pre_evals_inner(fused.fld, result)?;
                return self.collect_pre_evals_inner(fused.val, result);
            }
            let def_fn = self.data.def(*d_nr);
            if def_fn.rust().is_empty() {
                // User-defined function: pre-eval any Block or nested user-fn arguments
                // (both cause double-borrow of stores if left inline).
                // @P312 — also pre-eval any arg that READS a variable which another
                // arg passes as a fresh `&mut var_X` borrow; native otherwise emits
                // `f(&mut var_x, g(var_x))` and the Copy-read of `var_x` while it is
                // mutably borrowed trips rustc E0503.
                let borrowed = self.borrowed_arg_vars(vals);
                for arg in vals {
                    let needs_pre = self.create_stack_var(arg).is_none()
                        && (Self::is_sequence_arg(arg)
                            || self.needs_pre_eval(arg)
                            || (!borrowed.is_empty()
                                && value_refs_any(IrNode::Native(arg), &borrowed)));
                    if needs_pre {
                        let name = format!("_pre_{}", self.counter);
                        self.counter += 1;
                        self.rewrite_code(result, arg, name, false)?;
                    } else {
                        self.collect_pre_evals_inner(arg, result)?;
                    }
                }
            } else {
                // Template function: pre-eval Block args (they may use stores) and,
                // when multiple user-fn args exist, pre-eval those too to avoid
                // double-borrow of stores.
                // `Insert` alongside `Block`: both are a SEQUENCE whose value is its tail,
                // and the user-fn branch above already treats them as one. Counting only
                // `Block` here left an `Insert` argument to fall through to the template's
                // own `let _haN = …` binder in `generation/calls.rs`, which is not braced —
                // so `let _ha0 = a; b; c` bound the FIRST statement (a `()` assignment) and
                // rustc rejected the use with E0609 (`no field 'rec' on type '()'`).
                // loft#1029 reached it by hoisting an argument's construction out of a call
                // written inside a formatted string, but nothing about that is special: any
                // `Insert` in an argument to a native template was mis-emitted.
                let block_count = vals.iter().filter(|a| Self::is_sequence_arg(a)).count();
                let user_fn_count = vals.iter().filter(|a| self.needs_pre_eval(a)).count();
                // Also pre-eval any arg whose template placeholder appears more than once
                // (e.g., `#rust"!@v1.is_nan() && ... @v1 ..."` expands @v1 twice, causing
                // double-borrow when @v1 is a user-fn call returning stores-backed data).
                let has_dup_param = def_fn.attributes().iter().enumerate().any(|(i, a)| {
                    let placeholder = format!("@{}", a.name);
                    i < vals.len()
                        && def_fn.rust().matches(placeholder.as_str()).count() > 1
                        && self.needs_pre_eval(&vals[i])
                });
                // Templates that touch `s.database.…`, `s.const_refs`, or
                // `s.string_from_const_store` get substituted to the
                // `stores` binding in native code (src/generation/calls.rs).
                // Treat any of these the same as an explicit `stores`
                // reference for pre-eval purposes so nested user-fn args
                // (which also take `&mut stores`) get hoisted into bindings
                // and avoid double-borrow.  Plan-07 phase 4 added more
                // `s.X` calls (`s.raise`, `s.vec_get_or_raise`,
                // `s.vec_ref_or_raise`, `s.text_char_or_raise`) that all
                // route to `stores` via `src/generation/calls.rs`'s
                // rewriter — register them here so wrap-emitting
                // contexts (e.g. `stores.scratch.push(...)` around a
                // text-returning call that uses these helpers) hoist
                // the inner call into a `let _pre_N = ...` binding
                // and avoid the rustc E0499 double-borrow.
                let template_uses_stores = def_fn.rust().contains("stores")
                    || def_fn.rust().contains("s.database.")
                    || def_fn.rust().contains("s.const_ref_at(")
                    || def_fn.rust().contains("s.string_from_const_store(")
                    || def_fn.rust().contains("s.raise(")
                    || def_fn.rust().contains("s.vec_get_or_raise(")
                    || def_fn.rust().contains("s.vec_ref_or_raise(")
                    || def_fn.rust().contains("s.text_char_or_raise(");
                let needs_pre_eval_args = block_count > 0
                    || user_fn_count > 1
                    || (template_uses_stores && user_fn_count > 0)
                    || has_dup_param;
                if needs_pre_eval_args {
                    for (arg_idx, arg) in vals.iter().enumerate() {
                        let is_block = Self::is_sequence_arg(arg);
                        let is_multi_user_fn = user_fn_count > 1 && self.needs_pre_eval(arg);
                        let is_stores_conflict = template_uses_stores && self.needs_pre_eval(arg);
                        let is_dup = if arg_idx < def_fn.attributes().len() {
                            let placeholder = format!("@{}", def_fn.attributes()[arg_idx].name);
                            def_fn.rust().matches(placeholder.as_str()).count() > 1
                                && self.needs_pre_eval(arg)
                        } else {
                            false
                        };
                        if is_block || is_multi_user_fn || is_stores_conflict || is_dup {
                            let name = format!("_pre_{}", self.counter);
                            self.counter += 1;
                            self.rewrite_code(result, arg, name, is_dup)?;
                        } else {
                            self.collect_pre_evals_inner(arg, result)?;
                        }
                    }
                } else {
                    for arg in vals {
                        self.collect_pre_evals_inner(arg, result)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Is this argument a SEQUENCE — a value whose Rust rendering is more than one
    /// statement, so it has to be lifted into a `let _pre_N = { … }` binding before the
    /// call rather than written inside the argument list?
    ///
    /// `Block` and `Insert` are the two, and they are one question: both render as
    /// `op; op; op` with the tail as the value. Left inline they either double-borrow
    /// `stores` or — for a native template, whose own `let _haN = @v1;` binder in
    /// `generation/calls.rs` is not braced — bind only the FIRST statement, which is an
    /// assignment, so the use site reads a field of `()` and rustc rejects it with E0609.
    ///
    /// ⚠ Read through the SPAN. A span is source position, not structure, and the parser
    /// leaves one on an argument it rewrote in place — which is exactly the shape
    /// loft#1029's argument hoist produces. Matching the bare variant made the lift
    /// depend on whether the value happened to carry a position, and one native template
    /// (`OpGetInt`) then received a spanned `Insert` and mis-emitted it.
    /// Does this argument's emitted Rust hand back an OWNED text — a `Str` or a `String` —
    /// where the callee's parameter is `&str`?
    ///
    /// Three shapes do, and they are the same shape at different depths, which is why
    /// this asks about the TAIL rather than about the outermost node: a text-returning
    /// USER function produces a `Str`; a value BLOCK yields its declared result; and a
    /// SEQUENCE yields its last operator, which is typically one of the other two.
    ///
    /// The sequence case is the one that was missing.  `print("{f([\"aa\"])}")`, where
    /// `f(v: vector<text>) -> text { v[0] }`, hoists the vector's CONSTRUCTION and the
    /// call together into one `_pre_N`, so the node is an `Insert` and neither of the
    /// two shapes that were listed matched — `--native` then handed a `String` to
    /// `format_text` and rustc refused the program (`expected &str, found String`) while
    /// the interpreter ran it.  Binding the argument first was the difference, because
    /// then there was nothing to hoist.
    ///
    /// `unspan` on the way in for the same reason: a `Span` wrapper is not a different
    /// value, and matching the bare node treated it as one.
    ///
    /// Over-applying the deref cannot be silent — an extra `&*` on something already
    /// borrowed is a rustc error, not a wrong answer.
    fn yields_owned_text(&self, arg: &Value) -> bool {
        match arg.unspan() {
            Value::Call(d, _) => {
                matches!(self.data.def(*d).returned(), Type::Text(_))
                    && self.data.def(*d).rust().is_empty()
                    && !self.data.def(*d).name().starts_with("Op")
            }
            Value::Block(b) => matches!(b.result, Type::Text(_)),
            Value::Insert(ops) => ops.last().is_some_and(|tail| self.yields_owned_text(tail)),
            _ => false,
        }
    }

    fn is_sequence_arg(arg: &Value) -> bool {
        matches!(arg.unspan(), Value::Block(_) | Value::Insert(_))
    }

    /// Use this to register one pre-eval binding: generate the expression text with inner
    /// pre-evals already substituted, then push `(name, code)` onto `result`.
    fn rewrite_code(
        &mut self,
        result: &mut PreEvalSet,
        arg: &Value,
        name: String,
        replace_all: bool,
    ) -> std::io::Result<()> {
        // Collect inner pre-evals first, so the pre-eval code itself
        // is free of double borrows.
        let decl_clone = self.declared.clone();
        let start_idx = result.entries.len();
        self.collect_pre_evals_inner(arg, result)?;
        // Propagate replace_all flag: if this pre-eval is a dup-param (replace_all=true),
        // all its inner pre-evals must also use replace_all so that progressive substitution
        // correctly transforms all N occurrences of the dup arg in the outer expression.
        if replace_all {
            for entry in &mut result.entries[start_idx..] {
                entry.4 = true;
            }
        }
        let inner_pre_evals = result.entries[start_idx..].to_vec();
        // Save counter state before generating the expression text;
        // output_block will restore to this value before output_code_with_subst
        // so the block inner pre-eval names (_pre_N) match in both passes.
        let counter_before_gen = self.counter;
        // Generate this pre-eval's own text with the ALREADY-COLLECTED bindings installed as
        // the identity map, so an inner hoisted node emits its `_pre_N` name instead of being
        // regenerated inline.  This is the mechanism the emit walk already uses
        // (`Output::active_pre_eval`, whose own comment says "keyed on the node's address
        // (intrinsic identity) — no counter, no regenerated-text match"); collection was the
        // one walk still matching TEXT, and text cannot carry identity across two generations
        // whose counters differ.
        //
        // loft#1505 is what that cost: the same node was lifted twice — once inside this
        // pre-eval's own text as `_pre_26`, once again while the PARENT regenerated the same
        // subtree as `_pre_27` — so the parent's `s.replace(<1935-byte key>, "_pre_25")` found
        // nothing, the binding stayed dead, and the expression was emitted a second time at
        // the use site.  A dead `let _pre_N = <a read>;` costs nothing, which is why this was
        // invisible until the lifted expression ALLOCATED and DROPPED: `OpDrop` then ran on the
        // displaced copy as well as the survivor, twice on `--native` against once on
        // `--interpret`.
        //
        // The node being generated is removed from the map: it is a key in `result` by the time
        // an enclosing call reaches here, and substituting it would emit `let _pre_N = _pre_N;`.
        let saved_ident = std::mem::replace(&mut self.active_pre_eval, {
            let mut m = result.name_map();
            m.remove(&node_id(arg));
            m
        });
        let raw_code = self.generate_expr_buf(arg)?;
        self.active_pre_eval = saved_ident;
        let substituted = if inner_pre_evals.is_empty() {
            raw_code
        } else {
            let mut s = raw_code;
            for (pre_name, pre_code, _, _, inner_replace_all) in &inner_pre_evals {
                if *inner_replace_all {
                    // Dup-param inner pre-eval: the arg code appears multiple times
                    // in the binding code (template expanded @v1 twice), replace all.
                    s = s.replace(pre_code.as_str(), pre_name.as_str());
                } else {
                    // Normal inner pre-eval: appears once, use replace-first.
                    s = s.replacen(pre_code.as_str(), pre_name.as_str(), 1);
                }
            }
            s
        };
        // When the argument type is a narrow integer (u8/u16/i8/i16), the Rust binding
        // would have a narrow type.  Post-2c pre-eval bindings must have type i64 so
        // they compare correctly against i64 expressions.  Compute a separate bind_code
        // that wraps the expression with `as i64`; the match_code (used for substitution)
        // is left unchanged so string replacement in the outer code still works.
        // The binding is emitted into `let _pre_N = …;`, so whatever the lowering
        // produced has to be ONE expression there.  A statement sequence becomes one by
        // being braced; every other form is already an expression and is left alone,
        // because a blanket brace would scope-limit a `&…` result to the block and leave
        // the binding pointing at a dropped temporary.  Only the BINDING text is braced —
        // `substituted` stays verbatim, because it is also the key an enclosing binding
        // string-matches its inner pre-evals by, and a braced key matches nothing.
        let bound = if is_statement_sequence(&substituted) {
            format!("{{ {substituted} }}")
        } else {
            substituted.clone()
        };
        let bind_code = if !substituted.is_empty() && substituted != "()" {
            if let Some(tp) = self.infer_type(IrNode::Native(arg)) {
                if narrow_int_cast(&tp).is_some() {
                    // Braces, not parens: `substituted` can be a STATEMENT sequence, not just an
                    // expression — e.g. a boolean `&&`/`||` lowering whose operand lifted a
                    // value-struct-returning call emits `<lift>; if <pred> {…} else {…}`. Wrapping
                    // that as `( stmt; expr ) as i64` is invalid Rust (a "found `;`" syntax error,
                    // then a bogus `((), u8) as i64`). A block `{ … } as i64` is valid for a single
                    // expression AND a statement sequence, so it works in every case.
                    format!("{{ {substituted} }} as i64")
                } else if matches!(tp, Type::Text(_)) && self.yields_owned_text(arg) {
                    // The binding holds an owned text where the callee's parameter is
                    // `&str`, so deref at the binding site.  See `yields_owned_text` for
                    // which shapes those are and why asking about the TAIL is what makes
                    // the list total.
                    format!("&*({bound})")
                } else {
                    bound.clone()
                }
            } else {
                bound.clone()
            }
        } else {
            bound.clone()
        };
        if !substituted.is_empty() && substituted != "()" {
            // Key the binding on `arg`'s intrinsic identity so the emit walk can
            // re-find it by node, not by regenerated text (see PreEvalSet).
            result.push(
                arg,
                (
                    name,
                    substituted,
                    bind_code,
                    counter_before_gen,
                    replace_all,
                ),
            );
        }
        self.declared = decl_clone;
        Ok(())
    }

    /// Use this to determine whether a value produces no Rust result (type `()`).
    /// Needed by `output_block` to find the last non-void expression that should be the
    /// block's return value.
    pub(super) fn is_void_value(&self, v: &Value) -> bool {
        match v {
            // N8a.2: TuplePut is an assignment statement (void), not a return expression.
            Value::Null
            | Value::Drop(_)
            | Value::Set(_, _)
            | Value::Line(_)
            | Value::TuplePut(_, _, _) => true,
            // An `if … else { null }` is a void STATEMENT only when its TAKEN
            // branch also yields no value — it diverges (`return …`) or is a
            // void / Never block (the enum-dispatch shape
            // `if tag==a { return … } else { null }`).  When the taken branch
            // PRODUCES a value (e.g. `if c { …; r } else { null }` of a
            // ref/struct type) the `if/else` IS the block's nullable return
            // value, not a statement — classifying it void let
            // `detect_ref_tail_capture` skip it, so the value was emitted as a
            // discarded statement and the trailing `Return(Null)` returned the
            // null SENTINEL (the imaging `png()` width=null bug under --native).
            Value::If(_, true_v, false_v) => {
                matches!(**false_v, Value::Null)
                    && match &**true_v {
                        Value::Return(_) => true,
                        Value::Block(bl) => matches!(bl.result, Type::Void | Type::Never),
                        other => self.is_void_value(other),
                    }
            }
            Value::Call(d_nr, _) => {
                let def = self.data.def(*d_nr);
                matches!(def.returned(), Type::Void)
            }
            Value::Block(bl) => matches!(bl.result, Type::Void),
            _ => false,
        }
    }

    /// Detect the native-only ref-return tail-call capture pattern.
    ///
    /// Matches ref/vector/struct-enum-returning blocks whose tail is:
    /// `[..., Call(user_fn -> matching ref type), cleanup_ops*, Return(Null)]`
    /// where `cleanup_ops` is zero or more of `OpFreeText`, `OpFreeRef`,
    /// `n_set_store_lock`, or `Value::Line`.
    ///
    /// Without the capture, `scopes::free_vars`'s else-branch produces
    /// `[Call, free, Return(Null)]` — native codegen then emits the Call
    /// as a discarded statement and returns a null DbRef sentinel,
    /// losing the tail call's result (`tests/scripts/87-store-leaks.loft`).
    ///
    /// The capture lets `output_block` emit
    /// `let __native_tail_ret: DbRef = <call>;` at the Call position and
    /// `return __native_tail_ret;` in place of the Return(Null), preserving
    /// the original execution order (Call runs before cleanup runs before
    /// the typed return).  The scopes-level B5-L3 wrap was reverted in
    /// `ef6a32b` because it broke `brick_buster_yield_resume` via the
    /// interpreter's deep-copy path; this emit-time fix sidesteps that.
    pub(super) fn detect_ref_tail_capture(
        &self,
        bl: &Block,
        operators: &[Value],
    ) -> Option<(usize, usize)> {
        // The capture's purpose is to preserve a tail Call's heap-typed
        // return value when intervening cleanup ops would otherwise force
        // it through `[Call; cleanup; return null]` (which discards the
        // value).  Both the block result type and — for `Type::Never`
        // blocks (an unconditional `return ...;` arm of an if/match) —
        // the enclosing function's return type qualify; the latter is
        // the @P274 path where `parse_append_text` (in vectors.rs) keeps
        // intervening OpFreeRef ops in place rather than allowing
        // `patch_hoisted_returns` to inline the Call into the Return.
        let target_type: &Type = match &bl.result {
            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => &bl.result,
            Type::Never => {
                let fn_ret = self.data.def(self.def_nr).returned();
                match fn_ret {
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => fn_ret,
                    _ => return None,
                }
            }
            _ => return None,
        };
        // Plan-11 (P204) fix: operators may be wrapped in `Value::Span(b)`
        // (position-tagging wrapper).  Match against the unspanned value
        // so the walker doesn't bail on Span-wrapped Calls.
        let ret_idx = operators
            .iter()
            .rposition(|op| !matches!(op.unspan(), Value::Line(_)))?;
        let Value::Return(ret_val) = operators[ret_idx].unspan() else {
            return None;
        };
        // The placeholder return is normally `Return(Null)`.  But when the
        // block's last op was itself a scope-exit free, `scopes::insert_free`
        // wraps THAT in the return: `Return(OpFreeText(j))` (a struct-returning
        // `match … { … => null }` whose value arm holds a heap local).  Treat that
        // free-wrapped return the same way — it's a void cleanup standing in for
        // the value, so capture the real preceding value; `output_block` emits
        // the inner free before `return __native_tail_ret`.
        if !matches!(*ret_val.unspan(), Value::Null)
            && Self::free_op_var(ret_val.unspan(), self.data).is_none()
        {
            return None;
        }
        // Walk backwards through cleanup ops to find the tail user Call.
        let mut i = ret_idx;
        while i > 0 {
            i -= 1;
            match operators[i].unspan() {
                Value::Line(_) => {}
                Value::Call(d_nr, _) => {
                    let name = self.data.def(*d_nr).name();
                    // Cleanup window: every free flavour (via the one
                    // recognizer — the old name list missed
                    // `OpFreeRefIfDistinct`, and the `Op*` fall-through
                    // below then ABORTED the tail capture) + the lock op.
                    if Self::free_op_var(&operators[i], self.data).is_some()
                        || name == "n_set_store_lock"
                    {
                        continue;
                    }
                    // Candidate tail Call — require its return type to match
                    // the target heap shape.  Only user-level functions
                    // (not raw `Op*` or loft-builtin calls) qualify.
                    if name.starts_with("Op") {
                        return None;
                    }
                    let callee_ret = self.data.def(*d_nr).returned();
                    if !self.heap_shape_matches(callee_ret, target_type) {
                        return None;
                    }
                    return Some((i, ret_idx));
                }
                // A value-producing tail expr (an `if … else { null }` whose
                // taken branch yields a value, etc.) is the block's nullable
                // heap result.  Capture it into `__native_tail_ret` BEFORE the
                // cleanup frees — exactly as the Call case — which preserves
                // order and avoids the use-after-free that blocks
                // `patch_hoisted_returns` (the value-`if` reads a work buffer a
                // cleanup op frees).  Without it the value is a discarded
                // statement and the trailing `Return(Null)` returns the null
                // sentinel (the fn always returned null under --native; imaging
                // `png()` width=null).  The `heap_shape_matches` guard (via
                // `infer_type`) mirrors the Call arm so the emitted
                // `let __native_tail_ret: DbRef = <tail>` stays well-typed;
                // `is_void_value` excludes diverging/void tails (enum-dispatch).
                other
                    if !self.is_void_value(other)
                        && self
                            .infer_type(IrNode::Native(other))
                            .is_some_and(|t| self.heap_shape_matches(&t, target_type)) =>
                {
                    return Some((i, ret_idx));
                }
                _ => return None,
            }
        }
        None
    }

    fn heap_shape_matches(&self, callee_ret: &Type, block_result: &Type) -> bool {
        match (callee_ret, block_result) {
            (Type::Reference(d1, _), Type::Reference(d2, _)) => d1 == d2,
            (Type::Vector(d1, _), Type::Vector(d2, _)) => d1 == d2,
            (Type::Enum(d1, true, _), Type::Enum(d2, true, _)) => d1 == d2,
            // A nullable-enum target also accepts a value of ONE OF ITS VARIANTS:
            // the tail `if cond { Variant{..} } else { null }` infers to the
            // variant's ref/enum type (e.g. `Circle`), whose parent def is the
            // enum (`Shape`).  Without this the tail capture misses, the present
            // value is dropped, and the native fn always returns the null sentinel.
            (Type::Reference(d1, _) | Type::Enum(d1, _, _), Type::Enum(d2, true, _)) => {
                // `parent()` is `u32::MAX` for a non-variant def; guard so a
                // parentless ref never spuriously matches a `u32::MAX` target.
                let parent = self.data.def(*d1).parent();
                d1 == d2 || (parent != u32::MAX && parent == *d2)
            }
            _ => false,
        }
    }
}
