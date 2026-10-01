// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! An `if`: scan both arms, reconcile what each owns, and establish the slots an arm assigns at
//! the parent scope.

use super::Scopes;
use super::backings::bind_backing_of;
use crate::data::{Data, Type, Value, v_set};
use crate::fxhash::FxHashMap as HashMap;
use crate::variables::Function;

/// Does the VALUE `val` yields name `v` — the tail of a value block, of either arm of a value
/// `if`, or of an insert?  A statement block yields nothing, so a mention in its last
/// statement is not a hand-out.
fn leaves_as_value(val: &Value, v: u16) -> bool {
    match val.unspan() {
        Value::Block(bl) => {
            !matches!(bl.result.base(), Type::Void)
                && bl
                    .operators
                    .last()
                    .is_some_and(|last| mentions_of(last, v) > 0)
        }
        Value::Insert(ops) => ops.last().is_some_and(|last| leaves_as_value(last, v)),
        Value::If(_, t, f) => leaves_as_value(t, v) || leaves_as_value(f, v),
        _ => false,
    }
}

/// Does a `return` inside `node` name `v`?
fn returned_in(node: &Value, v: u16) -> bool {
    if let Value::Return(value) = node.unspan()
        && mentions_of(value, v) > 0
    {
        return true;
    }
    let mut found = false;
    node.for_each_child(&mut |c| found = found || returned_in(c, v));
    found
}

/// How many nodes of `node` name `v` — [`var_mentions_in`]'s count for one variable.
fn mentions_of(node: &Value, v: u16) -> usize {
    let named = match node.unspan() {
        Value::Var(x)
        | Value::Set(x, _)
        | Value::TupleGet(x, _)
        | Value::TuplePut(x, _, _)
        | Value::FnRefDnr(x)
        | Value::CallRef(x, _)
        | Value::Iter(x, _, _, _) => *x == v,
        Value::FnRef(_, x, _) => *x != u16::MAX && *x == v,
        _ => false,
    };
    let mut n = usize::from(named);
    node.for_each_child(&mut |c| n += mentions_of(c, v));
    n
}

/// The value of `v`'s one `Set` in the two arms of an `if`, where `v` is assigned under no
/// loop and under no other name, or `None`.  `scan_if` reads it together with
/// `multi_assigned` (the whole body's count), so a `Some` is the variable's ONLY bind.
fn only_bind_in_arms<'a>(t_val: &'a Value, f_val: &'a Value, v: u16) -> Option<&'a Value> {
    fn find<'a>(node: &'a Value, v: u16, out: &mut Vec<&'a Value>) {
        match node.unspan() {
            Value::Set(w, value) if *w == v => out.push(value),
            // A bind inside a loop runs again on the next pass, after a bind of its own.
            Value::Loop(_) => {}
            other => other.for_each_child(&mut |c| find(c, v, out)),
        }
    }
    let mut found = Vec::new();
    find(t_val, v, &mut found);
    find(f_val, v, &mut found);
    match found.as_slice() {
        [one] => Some(one),
        _ => None,
    }
}

/// The backing the ONE bind of `v` inside `arm` fills ([`bind_backing_of`]); `None` when the arm
/// binds `v` other than once or through another shape.
fn arm_bind_backing(arm: &Value, v: u16, function: &Function, data: &Data) -> Option<u16> {
    fn find<'a>(node: &'a Value, v: u16, out: &mut Vec<&'a Value>) {
        if let Value::Set(w, value) = node.unspan()
            && *w == v
        {
            out.push(value);
        }
        node.for_each_child(&mut |c| find(c, v, out));
    }
    let mut found = Vec::new();
    find(arm, v, &mut found);
    match found.as_slice() {
        [one] => bind_backing_of(one, function, data),
        _ => None,
    }
}

impl Scopes<'_> {
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn scan_if(
        &mut self,
        test: &Value,
        t_val: &Value,
        f_val: &Value,
        function: &mut Function,
        data: &Data,
    ) -> Value {
        // Find Reference/Vector/Text variables first assigned inside either branch
        // (including nested ifs, but not inside loops).
        let mut pre_inits: Vec<u16> = Vec::new();
        self.find_first_ref_vars(t_val, function, &mut pre_inits);
        self.find_first_ref_vars(f_val, function, &mut pre_inits);
        // `(B-Scope)`: a name whose earlier binding's scope has ENDED is a new binding here,
        // and an arm that opens no scope of its own (a lowering's `Insert` or bare `Set`, the
        // `??` bind written into its arms) makes it at THIS `if`'s scope — so it is split
        // here and pre-inited like a fresh name (loft#1691).  Split inside the arm instead,
        // both arms named one copy that `--native` declared in the first arm.
        let mut splits: Vec<u16> = Vec::new();
        self.find_split_first_vars(t_val, function, &mut splits);
        self.find_split_first_vars(f_val, function, &mut splits);
        for ov in splits {
            let nv = function.copy_variable(ov);
            self.var_mapping.insert(ov, nv);
            pre_inits.push(nv);
        }
        // The arm-scoped ones are kept aside: each is its arm's own local, so its one bind
        // there is a FIRST bind on every path — the adoption below offers it the same.
        let mut arm_scoped: Vec<u16> = Vec::new();
        if crate::keys::arm_scope_enabled() {
            pre_inits.retain(|&v| {
                let confined = self.confined_to_one_arm(v, t_val, f_val, function, data);
                if confined {
                    arm_scoped.push(v);
                }
                !confined
            });
        }

        // Also find small variables assigned in BOTH branches (or an else-if chain).
        let mut small_both: Vec<u16> = Vec::new();
        let mut t_vars: Vec<u16> = Vec::new();
        let mut f_vars: Vec<u16> = Vec::new();
        Self::find_assigned_vars(t_val, &self.var_mapping, &mut t_vars);
        Self::find_assigned_vars(f_val, &self.var_mapping, &mut f_vars);
        for &v in &t_vars {
            if f_vars.contains(&v)
                && !self.var_scope.contains_key(&v)
                && !pre_inits.contains(&v)
                && !needs_pre_init(function.tp(v))
            {
                small_both.push(v);
            }
        }

        // @PLN164 B1 behind a null-init (`@FR-O-Move`) — the pre-init below turns a local's
        // bind in this `if` into a REBIND, and a rebind copies where a first bind adopts.
        // When that bind is the local's only assignment it follows the pre-init on every
        // path that reaches it, so the local holds the sentinel there: it is a first bind.
        if crate::keys::adopt_first_bind_enabled() {
            // …and the same fact when the AUTHOR wrote the null: `(B-Scope)` refuses a read of
            // a local an arm first binds, so `x: T? = null; if c { x = mk() } … x` is how such a
            // local is spelled now, and every bind of it but the arm's writes the sentinel.
            let null_led: Vec<u16> = self
                .null_led
                .iter()
                .copied()
                .filter(|v| !pre_inits.contains(v))
                .collect();
            for &v in pre_inits
                .iter()
                .chain(null_led.iter())
                .chain(arm_scoped.iter())
            {
                if (!self.multi_assigned.contains(&v) || self.null_led.contains(&v))
                    && let Some(value) = only_bind_in_arms(t_val, f_val, v)
                    && crate::use_analysis::adopts_minted_at_bind(data, function, v, value)
                {
                    function.mark_deferred_first_bind(v);
                }
            }
        }
        // Register pre-inited vars in var_scope BEFORE scanning branches so that
        // the branch scans see them as already assigned and use the set_var/OpPutRef
        // re-assignment path instead of claim().
        for &v in &pre_inits {
            self.register_binding(v, function);
        }
        // Register small variables assigned in both branches at the parent scope too.
        for &v in &small_both {
            self.put_scope(v);
            self.var_order.push(v);
        }

        let scanned_test = self.scan(test, function, data);
        // #316 — ownership state is path-sensitive: scan each branch from the
        // same pre-If state, then keep only entries BOTH branches agree on.
        // Enforces @FR-O-Complete — the fact is per BINDING and per PATH, a
        // set-and-reconcile rather than one structural walk.  Both arms are scanned from
        // the SAME pre-`If` state and only what they AGREE on survives, so a store owned
        // on one path only is not treated as owned after the join.
        //
        // ⚠ @FR-O-Complete is the load-bearing invariant of the whole model: loft has no
        // user-facing borrow checker, so an incomplete fact is not a compile error someone
        // fixes — it is a miscompile or a leak.  Erring toward "not owned" here is why the
        // reconcile intersects rather than unions.
        let owned_before = self.owned_refs.clone();
        let backing_before = self.construction_backing.clone();
        let binds_before = self.bind_backing.clone();
        let views_before = self.view_backing.clone();
        let mints_before = self.tuple_call_mint.clone();
        let now_before = self.tuple_member_now.clone();
        let scanned_true = self.scan(t_val, function, data);
        let owned_after_true = std::mem::replace(&mut self.owned_refs, owned_before);
        let backing_after_true = std::mem::replace(&mut self.construction_backing, backing_before);
        let binds_after_true = std::mem::replace(&mut self.bind_backing, binds_before);
        let views_after_true = std::mem::replace(&mut self.view_backing, views_before);
        let mut mints_after_true = std::mem::replace(&mut self.tuple_call_mint, mints_before);
        let now_after_true = std::mem::replace(&mut self.tuple_member_now, now_before);
        let scanned_false = self.scan(f_val, function, data);
        self.owned_refs
            .retain(|k, depth| owned_after_true.get(k) == Some(depth));
        self.construction_backing
            .retain(|k, w| backing_after_true.get(k) == Some(w));
        self.bind_backing
            .retain(|k, w| binds_after_true.get(k) == Some(w));
        self.view_backing
            .retain(|k, b| views_after_true.get(k) == Some(b));
        // Two arms that agree a tuple member is MINTED and disagree only on WHICH claimant
        // minted it (`u = (mk(1), 0); if c { u = (mk(2), 5) }`) join to a sole owner: every
        // claimant is disarmed after the `if`, so the member's own release runs the hook on
        // whichever path ran.  A claimant's live record can only be that member — each is the
        // buffer of the one call, construction or move that filled it — so the disarm leaves
        // nothing unreleased.  Dropped from the join instead, the member fell to the bare
        // free on both paths and no hook ran (loft#1645).  A statement `if` only: after a
        // value `if` a statement would change its value.
        let statement_if = [&scanned_true, &scanned_false]
            .iter()
            .all(|arm| match arm.unspan() {
                Value::Null => true,
                Value::Block(b) => matches!(b.result.base(), Type::Void),
                _ => false,
            });
        let mut disarms: Vec<Value> = Vec::new();
        if statement_if {
            let mut joined: Vec<(u16, u16)> = Vec::new();
            for (k, m) in &self.tuple_call_mint {
                let Some(t) = mints_after_true.get(k) else {
                    continue;
                };
                if t == m {
                    continue;
                }
                for (idx, claim) in m {
                    if let Some(other) = t.get(idx)
                        && other != claim
                    {
                        joined.push((*k, *idx));
                        for w in [claim, other].into_iter().flatten() {
                            if !disarms
                                .iter()
                                .any(|d| matches!(d, Value::Set(x, _) if x == w))
                            {
                                disarms.push(v_set(
                                    *w,
                                    Value::Call(data.def_nr("OpNullRefSentinel"), vec![]),
                                ));
                            }
                        }
                    }
                }
            }
            joined.sort_unstable();
            for (k, idx) in joined {
                let (Some(m), Some(t)) = (
                    self.tuple_call_mint.get_mut(&k),
                    mints_after_true.get_mut(&k),
                ) else {
                    continue;
                };
                m.insert(idx, None);
                t.insert(idx, None);
            }
            disarms.sort_by_key(|d| match d {
                Value::Set(x, _) => *x,
                _ => u16::MAX,
            });
        }
        self.tuple_call_mint
            .retain(|k, m| mints_after_true.get(k) == Some(m));
        self.tuple_member_now
            .retain(|k, m| now_after_true.get(k) == Some(m));
        let scanned_if = Value::If(
            Box::new(scanned_test),
            Box::new(scanned_true),
            Box::new(scanned_false),
        );
        let scanned_if = if disarms.is_empty() {
            scanned_if
        } else {
            let mut stmts = vec![scanned_if];
            stmts.extend(disarms);
            Value::Insert(stmts)
        };

        if pre_inits.is_empty() {
            return scanned_if;
        }

        // Emit Set(v, Null/empty) for each variable at the current scope, before the
        // If node.  These are NOT passed through scan() again — the var_scope check
        // in the Set arm would strip them (contains_key + Null → Insert([])).
        let mut stmts: Vec<Value> = Vec::new();
        for &v in &pre_inits {
            if matches!(function.tp(v), Type::Text(_)) {
                stmts.push(v_set(v, Value::Text(String::new())));
            } else {
                stmts.push(v_set(v, Value::Null));
            }
        }
        match scanned_if {
            Value::Insert(ops) => stmts.extend(ops),
            other => stmts.push(other),
        }
        Value::Insert(stmts)
    }

    /// `@FR-H-Drop`'s scope-end clause for an `if` arm's owner: a local every mention of which,
    /// anywhere in the function, lies inside ONE arm of this `if` is that arm's local.  The
    /// arm's scan declares it there and it is released at the arm's end, where a pre-init at
    /// the `if`'s scope would release it at THAT scope's end.  The pre-init is what a local
    /// read after the `if`, or in the other arm, needs; any mention outside the arm keeps it.
    /// Judged only for a local the program declared — a compiler temp is used by lowerings
    /// after this pass, where the count cannot see it — and bound under its own number (a
    /// `var_mapping` copy is not in the body the count was taken from).  A local another
    /// variable's type depends on stays unless that variable is confined to the same arm too.
    fn confined_to_one_arm(
        &self,
        v: u16,
        t_val: &Value,
        f_val: &Value,
        function: &Function,
        data: &Data,
    ) -> bool {
        let total = self.mentions.get(&v).copied().unwrap_or(0);
        if total == 0
            || self.sunk.contains(&v)
            || function.is_compiler_generated(v)
            || self.var_mapping.contains_key(&v)
            || self.var_mapping.values().any(|m| *m == v)
        {
            return false;
        }
        // Every mention inside the two arms.  Bound in BOTH, the local is each arm's own — but a
        // collection or a tuple releases through the backing its type names, one for the
        // variable, so the arm that bound another backing would release the wrong one.
        let (in_t, in_f) = (mentions_of(t_val, v), mentions_of(f_val, v));
        let collection = matches!(function.tp(v).base(), Type::Vector(_, _) | Type::Tuple(_));
        // A VECTOR bound in both arms is each arm's own when each arm's bind names the backing it
        // fills (`bind_backing`, loft#1607): the arm's release then reads that one.
        let per_arm_backing = matches!(function.tp(v).base(), Type::Vector(_, _))
            && arm_bind_backing(t_val, v, function, data).is_some()
            && arm_bind_backing(f_val, v, function, data).is_some();
        if in_t + in_f != total || (collection && in_t > 0 && in_f > 0 && !per_arm_backing) {
            return false;
        }
        // A local the arm hands OUT — as the arm's value (`w = if c { a = …; a } else { … }`) or
        // through a `return` — leaves the arm, so the arm's end is not its owner's death.
        if leaves_as_value(t_val, v)
            || leaves_as_value(f_val, v)
            || returned_in(t_val, v)
            || returned_in(f_val, v)
        {
            return false;
        }
        (0..function.count()).all(|x| {
            x == v
                || !function.tp(x).depend().contains(&v)
                || self
                    .mentions
                    .get(&x)
                    .is_none_or(|&n| mentions_of(t_val, x) + mentions_of(f_val, x) == n)
        })
    }

    /// Collect the variables an `if` branch ASSIGNS, so the caller can emit their
    /// null-init before the `If` — a variable written in one arm and read after it must
    /// hold null rather than an uninitialised slot.
    ///
    /// ⚠ `unspan()` first — the requirement `Value::unspan`'s own doc states for every site
    /// that pattern-matches a specific variant.  Without it a `Span` wrapping a `Set` falls
    /// to the catch-all, the assignment is not collected, and the variable loses its init.
    ///
    /// That path is REACHED: instrumented over 200 corpus programs, the catch-all dropped
    /// 2 Span-wrapped `Set`s and 8 whole Span-wrapped `Block`s.  No program's IR changes
    /// when the peel is added, so nothing observable was riding on it — the vars in
    /// question were already covered another way.  The peel stays because the reachability
    /// is what makes it a trap: Span placement has moved before, and the failure mode is a
    /// missing initialisation with nothing to report it.
    fn find_assigned_vars(val: &Value, mapping: &HashMap<u16, u16>, result: &mut Vec<u16>) {
        match val.unspan() {
            Value::Set(v, inner) => {
                let resolved = *mapping.get(v).unwrap_or(v);
                if !result.contains(&resolved) {
                    result.push(resolved);
                }
                Self::find_assigned_vars(inner, mapping, result);
            }
            Value::Block(bl) => {
                for op in &bl.operators {
                    Self::find_assigned_vars(op, mapping, result);
                }
            }
            Value::If(c, t, f) => {
                Self::find_assigned_vars(c, mapping, result);
                Self::find_assigned_vars(t, mapping, result);
                Self::find_assigned_vars(f, mapping, result);
            }
            Value::Insert(ops) => {
                for op in ops {
                    Self::find_assigned_vars(op, mapping, result);
                }
            }
            Value::Call(_, args) | Value::CallRef(_, args) => {
                for a in args {
                    Self::find_assigned_vars(a, mapping, result);
                }
            }
            Value::Drop(inner) | Value::Return(inner) => {
                Self::find_assigned_vars(inner, mapping, result);
            }
            _ => {}
        }
    }

    /// Recursively collect variables that need a pre-init `Set(v, Null)` before an if/else.
    ///
    /// A variable is collected when it:
    /// - appears as the target of `Value::Set(v, ...)`,
    /// - has not yet been assigned (`var_scope` does not contain it), and
    /// - owns its allocation (`needs_pre_init` returns true).
    ///
    /// Recurses into nested `If` and `Block` but NOT into `Loop` — loop variables have
    /// per-iteration scope management and must not be pre-inited at the enclosing scope.
    fn find_first_ref_vars(&self, val: &Value, function: &Function, result: &mut Vec<u16>) {
        // Peeled for the same reason as its sibling [`Self::find_assigned_vars`], which
        // `scan_if` calls two lines below this one for the same job.  The arms discriminate
        // on `Set` / `Block` / `If` / `Insert`, so a spanned one took `_ => {}` — contributing
        // nothing for that whole subtree, where the shape being looked for is a branch's
        // FIRST assignment of a Reference/Vector/Text, i.e. the thing that decides pre-init.
        //
        // Reachable, and measured latent.  Over the 858-program corpus the peel changes the
        // decision at 46 sites in 16 programs, and at every one of them it newly
        // pre-initialises **0** variables: the same variables were already reaching `result`
        // by another path.  So no emitted code moves today.  The peel stays because it is one
        // word and it obeys `Value::unspan`'s documented rule, and because the reachability is
        // what makes it a trap — span placement has moved before, and the failure mode here is
        // a missing initialisation with nothing to report it.  Claiming a defect was fixed
        // would be the dressed-up version of this result.
        match val.unspan() {
            Value::Set(v, _) => {
                let resolved = *self.var_mapping.get(v).unwrap_or(v);
                // For borrowed types (non-empty dep), only pre-init if every dep is already
                // in var_scope — otherwise the OpCreateStack emitted at pre-init time would
                // reference an uninitialised slot.
                let deps_ready = function
                    .tp(resolved)
                    .depend()
                    .iter()
                    .all(|d| self.var_scope.contains_key(d));
                if !self.var_scope.contains_key(&resolved)
                    && needs_pre_init(function.tp(resolved))
                    && deps_ready
                    && !result.contains(&resolved)
                {
                    result.push(resolved);
                }
            }
            Value::Block(bl) => {
                for op in &bl.operators {
                    self.find_first_ref_vars(op, function, result);
                }
            }
            Value::If(_, t, f) => {
                self.find_first_ref_vars(t, function, result);
                self.find_first_ref_vars(f, function, result);
            }
            Value::Insert(ops) => {
                for op in ops {
                    self.find_first_ref_vars(op, function, result);
                }
            }
            // Do NOT recurse into Value::Loop — loop-interior Reference
            // variables are handled by the Loop handler in scan() which
            // pre-inits them at the pre-loop scope.
            _ => {}
        }
    }

    /// The names an `if` arm assigns at the `if`'s OWN scope — through `Insert` and nested
    /// `If` arms, never into a `Block` (which scopes its own binding) or a `Loop` — whose
    /// binding [`Self::scan_set`] would split, because the scope it was registered at has
    /// ended.  The same test `scan_set` makes, so the split made here is the one it reuses.
    fn find_split_first_vars(&self, val: &Value, function: &Function, result: &mut Vec<u16>) {
        match val.unspan() {
            Value::Set(ov, _) => {
                let dead = |v: &u16| {
                    self.var_scope
                        .get(v)
                        .is_some_and(|s| *s != self.scope && !self.stack.contains(s))
                };
                if dead(ov)
                    && self.var_mapping.get(ov).is_none_or(dead)
                    && needs_pre_init(function.tp(*ov))
                    && function
                        .tp(*ov)
                        .depend()
                        .iter()
                        .all(|d| self.var_scope.contains_key(d))
                    && !result.contains(ov)
                {
                    result.push(*ov);
                }
            }
            Value::If(_, t, f) => {
                self.find_split_first_vars(t, function, result);
                self.find_split_first_vars(f, function, result);
            }
            Value::Insert(ops) => {
                for op in ops {
                    self.find_split_first_vars(op, function, result);
                }
            }
            _ => {}
        }
    }
}

/// Does a variable of this type need its slot established at the PARENT scope
/// when an `if`/`else` assigns it in a branch?
///
/// The question is really "is this variable backed by a heap store".  One that
/// is cannot be treated like a scalar written in both arms: `scan_if` registers
/// such a scalar at the parent scope directly (`small_both`), and for a
/// store-backed variable that hoists the OWNERSHIP without ever creating the
/// store, so the scope-exit free meets a stack-record ref where it expects an
/// owned heap store.
///
/// The KEYED collections were missing, and every one of them crashed the
/// interpreter for it: two arms of one `if`/`else` chain declaring the same
/// `hash` / `sorted` / `index` name gave `BUG (#306): a stack-record ref was
/// treated as an owned heap store`, then a SIGSEGV once the branch appended.
/// `vector` was in the list and so was fine, which is why the fault looked
/// type-specific rather than like the omission it was.  `--native` computes the
/// answer separately and was always right, so nothing outside the interpreter
/// changed.
pub(super) fn needs_pre_init(tp: &Type) -> bool {
    // Through `base()`: a nullable `S?` / `vector<T>?` / `text?` local is the same slot
    // behind a nullability marker (`@FR-L-Null`), and it needs the same initialisation —
    // the null it holds on the path that never assigned it.  Matching the bare spelling
    // left the nullable twin with none: first assigned inside a branch, the second arm's
    // `Set` was a REASSIGNMENT whose guarded displacement free read an uninitialised
    // slot (a refused free of `0xDEADBEEF`, or the free of whatever live store the
    // previous frame left there); first assigned inside a loop body, it stayed scoped
    // to the body and the read after the loop was a use-after-free on the interpreter
    // and an unresolved `var_x` under rustc.  Both backends, every nullable kind.
    matches!(
        tp.base(),
        Type::Text(_) | Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
    ) || crate::parser::vectors::is_keyed(tp.base())
}
