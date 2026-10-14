// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! A value BRANCH's arm tails: each path of a branch whose value is bound gets a binding of its
//! own, so the joined binding has one ownership fact per path.

use super::Scopes;
use super::backings::{construction_work_ref, construction_work_refs};
use super::handoff::copy_moves_drop_from;
use super::join_rewrite::branch_tail_vars;
use crate::data::{Block, Data, Deps, Type, Value, v_set};
use crate::variables::Function;

/// Is `v` the temp a null DISCHARGE hoists its SUBJECT into?
///
/// `e ?? d` and `e ?? return` bind a non-trivial subject to a temp of their own and hand that
/// temp back from the tail `if`'s present arm, so it is the subject's value wearing a compiler
/// name.  Two questions read it — what the join BORROWS on that path
/// ([`Scopes::lift_arm_tails_into`]), and whether that arm needs a store of its own
/// ([`Scopes::arm_bind`]) — and one home keeps them from drifting apart.
pub(super) fn is_discharge_hoist(function: &Function, v: u16) -> bool {
    let name = function.name(v);
    name.starts_with("__ncc_") || name.starts_with("_ncr_")
}

/// The call a `join-arm-owner` block binds — `{ buf = call; buf }`, exactly as
/// `Parser::materialise_owned_call` builds it — or `None` for any other contents, which are then
/// not written out.
fn owner_block_call(bl: &Block) -> Option<&Value> {
    let [set, tail] = bl.operators.as_slice() else {
        return None;
    };
    match (set.unspan(), tail.unspan()) {
        (Value::Set(buf, call), Value::Var(t)) if buf == t => Some(call),
        _ => None,
    }
}

/// What a value-branch arm tail is rewritten into ([`Scopes::arm_bind`]).
enum ArmBind {
    /// `{ __lift_N = <tail>; __lift_N }` — a temp of this type, bound by the single-bind
    /// lowering of the tail (adopt, copy, or `OpBindOrCopy`, whichever that bind does).
    Bind(Type),
    /// `{ OpReplaceVector(__lift_N, <var>, elem); __lift_N }` — a function-scoped vector
    /// buffer refilled from the variable, the copy `@FR-B-Copy` asks of a plain bind.
    CopyVector { tp: Type, elem: i32 },
}

/// Does `node` mention variable `v` outside the binds of the temps in `lifted` — i.e. is `v`
/// still read by an arm that was NOT copied into one of them?
fn mentions_var_outside(node: &Value, v: u16, lifted: &[u16]) -> bool {
    match node.unspan() {
        Value::Set(t, _) if lifted.contains(t) => return false,
        Value::Call(_, args) if matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if lifted.contains(t)) =>
        {
            return false;
        }
        Value::Var(x) if *x == v => return true,
        _ => {}
    }
    let mut found = false;
    node.for_each_child(&mut |c| {
        if !found && mentions_var_outside(c, v, lifted) {
            found = true;
        }
    });
    found
}

impl Scopes<'_> {
    /// `OpReplaceKeyed(<branch>, r, tp)` with per-arm temps lifted out of `<branch>`, or
    /// `None` where the value is not that op, its branch has no qualifying arm, or it was
    /// rewritten already (the arms then end in `Insert`s whose tails are temps, not calls).
    pub(super) fn rewrite_keyed_replace_branch(
        &mut self,
        val: &Value,
        function: &mut Function,
        data: &Data,
    ) -> Option<Value> {
        let Value::Call(d, args) = val.unspan() else {
            return None;
        };
        if data.def(*d).name() != "OpReplaceKeyed" || args.len() < 2 {
            return None;
        }
        let Value::Var(target) = args[1].unspan() else {
            return None;
        };
        if !Self::is_value_branch(&args[0])
            || !self.arm_tails_need_binding(&args[0], *target, data, function)
        {
            return None;
        }
        let home = self.var_scope.get(target).copied().unwrap_or(self.scope);
        let mut branch = args[0].clone();
        let _ = self.lift_join_arm_tails(&mut branch, home, *target, function, data);
        let mut new_args = args.clone();
        new_args[0] = branch;
        let call = Value::Call(*d, new_args);
        Some(match val {
            Value::Span(b) => Value::Span(Box::new((b.0, call))),
            _ => call,
        })
    }

    /// How many loops enclose scope `home` — the loop depth a binding declared THERE sees,
    /// as opposed to the depth at the current position.  `self.loops` holds the scope ids of
    /// the loops entered so far and `self.stack` the enclosing scopes outer-to-inner, so the
    /// loops that enclose `home` are those at or before it on that stack.
    fn loop_depth_at(&self, home: u16) -> usize {
        if home == self.scope {
            return self.loops.len();
        }
        match self.stack.iter().position(|&sc| sc == home) {
            Some(idx) => self
                .loops
                .iter()
                .filter(|&&l| l == home || self.stack[..idx].contains(&l))
                .count(),
            None => self.loops.len(),
        }
    }

    /// Is this RHS a BRANCH — an `if` expression, or a `match` lowered to a value block whose
    /// tail is the `if` chain?  Only a branch is rewritten: a bare call is the bound spelling
    /// already, and rewriting it into a bound temp would scan the same shape forever.
    /// The statement form of `Set(v, <value branch>)`: every arm tail `t` becomes `Set(ov, t)`
    /// and the arms stop yielding a value — or `None` where an arm tail is not one a plain
    /// bind can take as it is: the binding itself, a compiler temp (a `??` hoist, a lift, a
    /// literal's work-ref), or a shape this cannot read.  `ov` is the ORIGINAL id, so each
    /// sunk `Set` takes the same scope mapping the value form would have.
    ///
    /// An arm that is not a block of its own — a bare tail, or an `Insert` as the `??` and scalar
    /// `match` lowerings write it — is given one.  A sunk `Set` can take a snapshot of the record
    /// it displaces, and that temp is registered at the scope the `Set` runs in and released at
    /// that scope's exit.  Without a block that scope is the ENCLOSING one, whose exit also runs on
    /// the paths where this arm, and the temp's declaration with it, never ran.
    ///
    /// `beside_any` also writes out a call arm's owner block beside a PROJECTION arm.  The scan
    /// declines that shape — the analyses before it read the value form, so the written-out
    /// projection would lack the view facts its own assignment gets — and the rewrite ahead of a
    /// rescan accepts it (`rewrite_written_out`), because there those analyses run afterwards.
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn sink_set_into_arms(
        v: u16,
        ov: u16,
        value: &Value,
        function: &Function,
        data: &Data,
        beside_any: bool,
    ) -> Option<Value> {
        // The chain's own block is a UNIT as an ARM (see `sinkable`), so as the VALUE being
        // bound it is not a branch to sink into: `b = e ?? d` is already the statement a sunk
        // arm would write, and sinking it would rebuild the same `Set` around it forever.
        if matches!(value.unspan(), Value::Block(bl) if bl.name == "ncc") {
            return None;
        }
        fn sinkable(tail: &Value, v: u16, ov: u16, function: &Function, data: &Data) -> bool {
            match tail.unspan() {
                // The binding itself: written out, that arm is `v = v`, the identity (#330).
                Value::Var(x) if *x == v || *x == ov => true,
                Value::Var(x) => {
                    (*x as usize) < function.count() as usize && !function.is_compiler_generated(*x)
                }
                Value::Null | Value::Call(_, _) | Value::CallRef(_, _) => true,
                Value::If(_, t, f) => {
                    sinkable(t, v, ov, function, data) && sinkable(f, v, ov, function, data)
                }
                // A call arm the parser gave an owner so the VALUE form's join had one; written
                // out, the arm binds that call itself.
                Value::Block(bl) if bl.name == crate::parser::Parser::JOIN_ARM_OWNER => {
                    owner_block_call(bl).is_some()
                }
                // A struct literal's construction, which ends in the work-ref holding the finished
                // record.  Written out, the arm binds that construction whole and the binding
                // adopts the record, as a single bind of the literal does
                // (`construction_work_refs`).
                Value::Block(bl) if bl.name == "Object" => {
                    construction_work_ref(tail, function).is_some()
                }
                // loft#1612 — a `??` chain's own block, sunk as a UNIT.  Its tail is the
                // hoisted `__ncc_N` temp, which is compiler-generated and so not a tail a
                // plain bind may take; the BLOCK is, and written out the arm binds the join
                // exactly as a statement of its own would (`b = e ?? d`).  Left unsinkable it
                // declines the WHOLE sink, and then a chosen VARIABLE arm beside it records no
                // hand-off: `b = x ?? (c() ?? d())` released `x`'s record from `b` AND from
                // `x`, the second release on freed memory — `@FR-H-Move`, and a double release
                // only a type with an `OpDrop` hook can witness.
                //
                // Only where every arm of that chain is OWNED, which is the question
                // `ncc_arms_are_all_owned` already asks for `inline_struct_return`: an arm
                // that VIEWS a place the program can still reach owes the destination a COPY
                // (`@FR-B-Copy`), and a written-out `Set` binds what it is given — the copy is
                // the parser's to emit and it is not there to emit one here.  Without the
                // clause `x: H = if k > 0 { s.h ?? b } else { mk(3) }` aliased `b`, which
                // `ownership_drop_gate`'s copy census names.
                Value::Block(bl) if bl.name == "ncc" => {
                    !crate::keys::no_chain_arm_sink() && Scopes::ncc_arms_are_all_owned(bl, data)
                }
                Value::Block(bl) if !matches!(bl.result, Type::Void | Type::Null) => bl
                    .operators
                    .last()
                    .is_some_and(|l| sinkable(l, v, ov, function, data)),
                Value::Insert(ops) => ops
                    .last()
                    .is_some_and(|l| sinkable(l, v, ov, function, data)),
                _ => false,
            }
        }
        // A nested `if` is not wrapped: its own arms are, when `sink` reaches them.
        fn block_arm(arm: &mut Value) {
            if matches!(arm.unspan(), Value::Block(_) | Value::If(_, _, _)) {
                return;
            }
            let inner = std::mem::replace(arm, Value::Null);
            *arm = Value::Block(Box::new(Block {
                name: "sunk arm",
                operators: vec![inner],
                result: Type::Void,
                scope: 0,
                var_size: 0,
            }));
        }
        // An owner block is written out only beside LOCAL arms and `null`.  Beside a projection,
        // or a call answering a view (which the parser leaves unowned), that arm is a `(B-View)`
        // view, and written out it would lose the dep that keeps it one.  Asked only of a value
        // `sinkable` accepted, so every block on a tail path is already a value block.
        fn owners_beside_locals_only(node: &Value) -> bool {
            fn walk(n: &Value, owner: &mut bool, other: &mut bool) {
                match n.unspan() {
                    Value::Var(_) | Value::Null => {}
                    Value::If(_, t, f) => {
                        walk(t, owner, other);
                        walk(f, owner, other);
                    }
                    Value::Block(bl) if bl.name == crate::parser::Parser::JOIN_ARM_OWNER => {
                        *owner = true;
                    }
                    Value::Block(bl) => {
                        if let Some(last) = bl.operators.last() {
                            walk(last, owner, other);
                        }
                    }
                    Value::Insert(ops) => {
                        if let Some(last) = ops.last() {
                            walk(last, owner, other);
                        }
                    }
                    _ => *other = true,
                }
            }
            let (mut owner, mut other) = (false, false);
            walk(node, &mut owner, &mut other);
            !owner || !other
        }
        fn sink(node: &mut Value, v: u16, ov: u16) {
            // An arm that hands back the binding itself keeps the value it already holds: it
            // moves nothing, displaces nothing and releases nothing (`formal/heap.md` (H-Move)).
            if let Value::Var(x) = node.unspan()
                && (*x == v || *x == ov)
            {
                *node = Value::Insert(Vec::new());
                return;
            }
            if let Value::Block(bl) = node
                && bl.name == crate::parser::Parser::JOIN_ARM_OWNER
                && let Some(call) = owner_block_call(bl).cloned()
            {
                *node = Value::Set(ov, Box::new(call));
                return;
            }
            if let Value::Block(bl) = node
                && bl.name == "Object"
            {
                let construction = std::mem::replace(node, Value::Null);
                *node = Value::Set(ov, Box::new(construction));
                return;
            }
            // A `??` chain's block is the arm's VALUE, bound whole (loft#1612): descending
            // into it would bind its `__ncc_N` temp, which is the view the chain hands back
            // and not a value a bind may adopt.
            if let Value::Block(bl) = node
                && bl.name == "ncc"
            {
                let chain = std::mem::replace(node, Value::Null);
                *node = Value::Set(ov, Box::new(chain));
                return;
            }
            match node {
                Value::Span(b) => sink(&mut b.1, v, ov),
                Value::If(_, t, f) => {
                    sink(t, v, ov);
                    sink(f, v, ov);
                    block_arm(t);
                    block_arm(f);
                }
                Value::Block(bl) => {
                    if let Some(last) = bl.operators.last_mut() {
                        sink(last, v, ov);
                    }
                    bl.result = Type::Void;
                }
                Value::Insert(ops) => {
                    if let Some(last) = ops.last_mut() {
                        sink(last, v, ov);
                    }
                }
                tail => {
                    let t = std::mem::replace(tail, Value::Null);
                    *tail = Value::Set(ov, Box::new(t));
                }
            }
        }
        // A construction is an ARM, never the branch: written out on its own it would be the
        // bind it already is, and the scan would write it out again forever.
        if matches!(value.unspan(), Value::Block(bl) if bl.name == "Object")
            || !sinkable(value, v, ov, function, data)
            || (!beside_any && !owners_beside_locals_only(value))
        {
            return None;
        }
        let mut out = value.clone();
        sink(&mut out, v, ov);
        Some(out)
    }

    /// Is this value produced by a BRANCH — one whose paths may each need a binding of their
    /// own ([`Self::lift_join_arm_tails`])?
    ///
    /// A value block counts through its TAIL, and the tail is not always an `if`.  A
    /// `?? return` discharge leaves the absent path by an early return, so what remains is a
    /// statement `if` that diverges and then a bare `Var` naming the temp the block hoisted its
    /// subject into — one surviving arm, spelled without a join.  Read as "not a branch" that
    /// arm never reached [`Self::arm_bind`], so a discharged projection kept aliasing a
    /// container across a `remove` that renumbered it while the `??`-with-default spelling of
    /// the very same read materialised (loft#1401).  One arm is still a path.
    ///
    /// Only a tail naming a local the block ITSELF bound qualifies, never an arbitrary
    /// variable: the value has to be one computed here for a per-path binding to mean anything.
    pub(super) fn is_value_branch(node: &Value) -> bool {
        match node.unspan() {
            Value::If(_, _, _) => true,
            Value::Block(bl) if !matches!(bl.result, Type::Void | Type::Null) => {
                let tail = bl.operators.last();
                tail.is_some_and(Self::is_value_branch)
                    || matches!(tail.map(Value::unspan), Some(Value::Var(x))
                        if bl.operators.iter().any(|o|
                            matches!(o.unspan(), Value::Set(s, _) if s == x)))
            }
            _ => false,
        }
    }

    /// Does any arm of this value-yielding branch end in a tail that gets a binding of its
    /// own ([`Self::arm_bind`])?  The read-only twin of [`Self::lift_join_arm_tails`], so the
    /// RHS is cloned only when something in it will be rewritten.  A value branch is seen
    /// through its wrappers: a `Span`, a value `Block` (a plain arm, or a `scalar_match`
    /// behind its subject binding) and an `Insert` all yield their LAST operator.
    pub(super) fn arm_tails_need_binding(
        &mut self,
        node: &Value,
        bound: u16,
        data: &Data,
        function: &Function,
    ) -> bool {
        match node.unspan() {
            Value::If(_, t, f) => {
                self.arm_tails_need_binding(t, bound, data, function)
                    || self.arm_tails_need_binding(f, bound, data, function)
            }
            Value::Block(bl) if !matches!(bl.result, Type::Void | Type::Null) => bl
                .operators
                .last()
                .is_some_and(|l| self.arm_tails_need_binding(l, bound, data, function)),
            Value::Insert(ops) => ops
                .last()
                .is_some_and(|l| self.arm_tails_need_binding(l, bound, data, function)),
            Value::CallRef(_, _) | Value::Call(_, _) | Value::Var(_) => self
                .arm_bind(node, bound, data, function, &mut false)
                .is_some(),
            _ => false,
        }
    }

    /// `@FR-O-Complete` — give each path of a value branch its own binding.  Every arm tail
    /// [`Self::arm_bind`] answers for is rewritten into the BOUND spelling on a `__lift_N`
    /// temp — `{ __lift_N = <tail>; __lift_N }`, or a refill of a vector buffer — declared at
    /// `home`, the scope the joined binding lives in, so it outlives the arm and dies with the
    /// binding that borrows it.  The joined binding `bound` then borrows the temps: its dep
    /// list is rewritten to name them in place of the variables the arms copied, so the fact
    /// it carries is true on every path (a variable another arm still views stays).
    pub(super) fn lift_join_arm_tails(
        &mut self,
        node: &mut Value,
        home: u16,
        bound: u16,
        function: &mut Function,
        data: &Data,
    ) -> bool {
        let mut copied: Vec<(u16, u16)> = Vec::new();
        let mut viewed: Vec<u16> = Vec::new();
        let mut materialised = false;
        self.lift_arm_tails_into(
            node,
            home,
            bound,
            function,
            data,
            &mut copied,
            &mut viewed,
            &mut materialised,
        );
        // Each `__lift_N = a` is a whole-value copy the collector never saw when it first ran
        // (the lift is built after it), so the drop moves here by the same rule — and PER PATH
        // by construction, since these temps are one per arm.  Recording the temps is what lets
        // the collector reach the same answer when it meets these copies later, from inside the
        // arm.
        //
        // `@FR-H-Move` / `@FR-H-Drop` (loft#1617) — the lift holds the STRUCTURE on the path
        // that copied, so the lift is what releases it and the source is what stops.  The
        // copy is a move of a value this frame owns, and the hook belongs to the record the
        // new owner holds; stopping the LIFT instead left the hook on the source's record,
        // which the binding that views the lift has since written (`x.id = 7` read back as the
        // source's old value in the hook).  The count was right either way, which is why only
        // a cell that separates the two records sees it.  Per path, because the arm may not
        // run: the flag is `false` where it did not, and there the source still owes its
        // release.  Off a PARAMETER the copy stops itself, as before — the caller owns that
        // record, so neither the lift nor a flag may release it (`@FR-H-Drop`'s closing
        // clause).
        for &(src, tmp) in &copied {
            self.arm_lift_temps.insert(tmp);
            let Some(stopped) = copy_moves_drop_from(function, data, tmp, src, true) else {
                continue;
            };
            // A source that OUTLIVES the loop keeps its release: the lift is per ITERATION, so
            // running the hook there releases one record once a pass — `(H-Spent)`, the same
            // reason the per-arm write-out declines such a branch
            // ([`Self::source_outlives_loop`]).
            if stopped == src && !self.source_outlives_loop(src, function) {
                self.per_path_pairs.insert((tmp, src));
                self.mint_handoff_flag(function, src);
            } else {
                self.drop_transferred.insert(tmp);
            }
        }
        if bound == u16::MAX || (copied.is_empty() && viewed.is_empty()) {
            return materialised;
        }
        // A binding assigned elsewhere keeps the parser's fact: the runtime join bind copies
        // its arms there, and naming a hoist here would make it a borrow at every Set it has.
        if self.multi_assigned.contains(&bound) {
            return materialised;
        }
        // From here the binding BORROWS its per-arm temps and releases nothing, so an arm it was
        // owning has no owner left: a minting call's answer lives only in the value the join
        // hands over, because the call's `__ref_N` buffer starts as the null sentinel and a
        // callee handed null mints a store of its own.  Each such arm is given a temp too —
        // `x = a ?? mk(7)` then owns `mk(7)`'s store on the path that made it, exactly as the
        // `if` spelling of the same join does through its `join-arm-owner` block
        // (`formal/heap-history.md` D-heap-16).
        let mut owned: Vec<u16> = Vec::new();
        self.lift_owned_call_tails(node, home, function, data, &mut owned);
        // loft#1623 — every frame-owned temp this join's value may live in, recorded against the
        // binding that borrows them.  Three sources and the third is why this is a record rather
        // than a dep read: the arm lifts (`copied`), the minting-call arms this pass just gave a
        // temp (`owned`), and the arms the PARSER already owns, whose `join-arm-owner` `__ref_N`
        // the binding's dep list never names.
        let mut holders: Vec<u16> = copied.iter().map(|&(_, t)| t).collect();
        for w in owned
            .iter()
            .copied()
            .chain(construction_work_refs(node, function, data))
        {
            if !holders.contains(&w) {
                holders.push(w);
            }
        }
        if bound != u16::MAX && !holders.is_empty() {
            let slot = self.join_holders.entry(bound).or_default();
            for w in holders {
                if !slot.contains(&w) {
                    slot.push(w);
                }
            }
        }
        let mut deps: Vec<u16> = function.tp(bound).depend().clone();
        for tmp in owned {
            deps.push(tmp);
        }
        // A `??` hoist the join hands back as an arm is a binding the join BORROWS on that
        // path — say so, or the joined binding reads as owning what the hoist holds, and a
        // return of it cannot be seen to hand the hoist's store out.
        for x in viewed {
            if !deps.contains(&x) {
                deps.push(x);
            }
        }
        let lifted: Vec<u16> = copied.iter().map(|&(_, tmp)| tmp).collect();
        for &(src, _) in &copied {
            if !mentions_var_outside(node, src, &lifted) {
                deps.retain(|&d| d != src);
            }
        }
        for &(_, tmp) in &copied {
            if !deps.contains(&tmp) {
                deps.push(tmp);
            }
        }
        function.depend_on_all(bound, &deps);
        materialised
    }

    /// Does an arm of the value branch `value` hand back a variable declared OUTSIDE the
    /// innermost loop this statement runs in — or a parameter, which every loop is inside?
    ///
    /// Moving such a variable into the binding moves it once per iteration, so from the second
    /// iteration on the arm reads a name `(H-Spent)` has already spent.  Not a variable the
    /// loop's body refills on every pass (`x = a ?? mk(); a = x`): the next pass reads the new
    /// value, so that move is the one-pass move the written-out arms already decide.
    pub(super) fn arm_source_outlives_loop(&self, value: &Value, function: &Function) -> bool {
        branch_tail_vars(value)
            .iter()
            .any(|&src| self.source_outlives_loop(src, function))
    }

    /// Is `src` a variable declared OUTSIDE the innermost loop this statement runs in — or a
    /// parameter, which every loop is inside?
    ///
    /// The per-source half of [`Self::arm_source_outlives_loop`], and ONE home for it, because
    /// the two deciders that read it must agree: the per-arm write-out declines such a branch,
    /// and [`Self::lift_join_arm_tails`] keeps the release with the source for the same reason.
    /// A `(H-Spent)` name may be moved once, and a source outside the loop is moved once per
    /// ITERATION — so the second pass reads a name already spent, and whichever side the
    /// release is put on it runs once per pass over ONE record.  Keeping it with the source is
    /// the answer that releases once; moving it to the per-iteration lift doubles it
    /// (`ownership_drop_gate`'s `p_l1`).  Until the rules' error exists, this is the fallback
    /// both sites take.
    fn source_outlives_loop(&self, src: u16, function: &Function) -> bool {
        !self.loops.is_empty()
            && !function.is_compiler_generated(src)
            && self
                .var_scope
                .get(&src)
                .is_none_or(|&home| self.loop_depth_at(home) < self.loops.len())
            && !self
                .loop_refills
                .last()
                .is_some_and(|refilled| refilled.contains(&src))
    }

    /// Give every arm tail that is a bare call MINTING a record a `__lift_N` temp of its own —
    /// `{ __lift_N = <call>; __lift_N }`, declared at `home` — and collect the temps in `out`.
    ///
    /// For a join whose binding [`Self::lift_join_arm_tails`] has just turned into a borrow: the
    /// binding no longer releases what an arm hands it, so a store only that arm made needs an
    /// owner on its own path.  The temp is that owner, null on every path that took another
    /// arm.  An arm [`Self::arm_bind`] already lifted ends in its temp, and an arm the parser
    /// already gave an owner ends in that owner's `__ref_N`, so neither is a bare call here.
    fn lift_owned_call_tails(
        &mut self,
        node: &mut Value,
        home: u16,
        function: &mut Function,
        data: &Data,
        out: &mut Vec<u16>,
    ) {
        match node {
            Value::Span(b) => self.lift_owned_call_tails(&mut b.1, home, function, data, out),
            Value::If(_, t, f) => {
                self.lift_owned_call_tails(t, home, function, data, out);
                self.lift_owned_call_tails(f, home, function, data, out);
            }
            Value::Block(bl) if !matches!(bl.result, Type::Void | Type::Null) => {
                if let Some(last) = bl.operators.last_mut() {
                    self.lift_owned_call_tails(last, home, function, data, out);
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last_mut() {
                    self.lift_owned_call_tails(last, home, function, data, out);
                }
            }
            Value::Call(d, _) => {
                let def = data.def(*d);
                if !def.is_loft_defined() {
                    return;
                }
                let (returned, opt) = def.returned().peel_optional();
                let tp = match returned {
                    Type::Reference(r, _) => Type::Reference(*r, Deps::none()),
                    Type::Enum(r, true, _) => Type::Enum(*r, true, Deps::none()),
                    _ => return,
                };
                if !matches!(
                    crate::use_analysis::ownership_of(data, self.d_nr, node),
                    crate::use_analysis::Own::Owned
                ) {
                    return;
                }
                let tmp = self.new_lift_var(function, &Self::reopt(opt, tp));
                self.var_scope.insert(tmp, home);
                self.lift_decl_depth.insert(tmp, self.loop_depth_at(home));
                let tail = std::mem::replace(node, Value::Null);
                *node = Value::Insert(vec![v_set(tmp, tail), Value::Var(tmp)]);
                out.push(tmp);
            }
            _ => {}
        }
    }

    /// The walk behind [`Self::lift_join_arm_tails`]; `copied` collects `(source, temp)` for
    /// every VARIABLE arm that was copied into a temp, `viewed` every `??` hoist temp an arm
    /// hands back as it is.
    #[allow(clippy::too_many_arguments)]
    #[expect(clippy::too_many_lines, reason = "inherited")]
    fn lift_arm_tails_into(
        &mut self,
        node: &mut Value,
        home: u16,
        bound: u16,
        function: &mut Function,
        data: &Data,
        copied: &mut Vec<(u16, u16)>,
        viewed: &mut Vec<u16>,
        materialised: &mut bool,
    ) {
        match node {
            Value::Span(b) => {
                self.lift_arm_tails_into(
                    &mut b.1,
                    home,
                    bound,
                    function,
                    data,
                    copied,
                    viewed,
                    materialised,
                );
            }
            Value::If(_, t, f) => {
                self.lift_arm_tails_into(
                    t,
                    home,
                    bound,
                    function,
                    data,
                    copied,
                    viewed,
                    materialised,
                );
                self.lift_arm_tails_into(
                    f,
                    home,
                    bound,
                    function,
                    data,
                    copied,
                    viewed,
                    materialised,
                );
            }
            Value::Block(bl) if !matches!(bl.result, Type::Void | Type::Null) => {
                if let Some(last) = bl.operators.last_mut() {
                    self.lift_arm_tails_into(
                        last,
                        home,
                        bound,
                        function,
                        data,
                        copied,
                        viewed,
                        materialised,
                    );
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last_mut() {
                    self.lift_arm_tails_into(
                        last,
                        home,
                        bound,
                        function,
                        data,
                        copied,
                        viewed,
                        materialised,
                    );
                }
            }
            Value::CallRef(_, _) | Value::Call(_, _) | Value::Var(_) => {
                let src = if let Value::Var(x) = node {
                    Some(*x)
                } else {
                    None
                };
                if let Some(x) = src
                    && is_discharge_hoist(function, x)
                    && crate::data::is_dbref(function.tp(x).base())
                {
                    viewed.push(x);
                }
                match self.arm_bind(node, bound, data, function, materialised) {
                    Some(ArmBind::Bind(tp)) => {
                        let tmp = self.new_lift_var(function, &tp);
                        self.var_scope.insert(tmp, home);
                        self.lift_decl_depth.insert(tmp, self.loop_depth_at(home));
                        let tail = std::mem::replace(node, Value::Null);
                        *node = Value::Insert(vec![v_set(tmp, tail), Value::Var(tmp)]);
                        if let Some(src) = src {
                            copied.push((src, tmp));
                        }
                    }
                    Some(ArmBind::CopyVector { tp, elem }) => {
                        let tmp = self.new_buffer_var(function, &tp);
                        let tail = std::mem::replace(node, Value::Null);
                        *node = Value::Insert(vec![
                            Value::Call(
                                data.def_nr("OpReplaceVector"),
                                vec![Value::Var(tmp), tail, Value::Int(elem)],
                            ),
                            Value::Var(tmp),
                        ]);
                        if let Some(src) = src {
                            copied.push((src, tmp));
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// What gives this arm tail a binding of its own, or `None` where the arm keeps the value
    /// it has.  The answer is what the SINGLE bind `t = <tail>` would leave `t` holding, and
    /// the temp is bound by that single bind's own lowering — nothing here re-derives a copy:
    ///
    ///   * a fn-ref call — [`Self::arm_callref_lift_type`];
    ///   * a named call answering a RECORD the caller must COPY (`@FR-O-Move`: a borrowed or
    ///     `Join` return; codegen's `callee_of` arm is the copy) — an owned temp.  A named
    ///     call's OWNED record and every named COLLECTION return stay: the binding owns what
    ///     they hand it, and where the lift makes the binding a borrow instead,
    ///     [`Self::lift_owned_call_tails`] gives the record arm a temp of its own;
    ///   * a plain VARIABLE, which a plain bind COPIES (`@FR-B-Copy`) — a record temp bound
    ///     from it (codegen copies a same-struct `Var` bind on its first and every later
    ///     Set), or a vector buffer refilled from it (`OpReplaceVector`).  Not a compiler temp
    ///     — a `__lift_N` is this rewrite's own product, an `_elm_N` a slot inside a container
    ///     — except a null-discharge HOIST on a binding the walk has NAMED: there the temp
    ///     holds the subject projection (`@FR-B-View-Depth`) and copies exactly as the inline
    ///     spelling of it does.  Not the binding itself (`r = if c { r } else { … }`), whose
    ///     transition free already reads that it is read.  Not a keyed collection, which
    ///     `OpReplaceKeyed` copies whatever the arm.  Not a `&` binding, which has no
    ///     `Var`-copy lowering to hand the temp to.  A struct-`Enum` variable IS one: it is
    ///     the same heap record shape as a struct (`Type::heap_def_nr`), and both emitters
    ///     copy its `Var` bind.
    ///
    /// The fallback `None` is *"the join borrows this arm as it did"*: a literal or a
    /// comprehension owns through its per-site buffer, a projection is a view, and a shape
    /// this cannot read costs the alias or the leak it already had — never a free of a store
    /// the caller still names.
    fn arm_bind(
        &mut self,
        tail: &Value,
        bound: u16,
        data: &Data,
        function: &Function,
        materialised: &mut bool,
    ) -> Option<ArmBind> {
        match tail.unspan() {
            Value::CallRef(_, _) => self
                .arm_callref_lift_type(tail, data, function)
                .map(ArmBind::Bind),
            // A PROJECTION tail, when the joined binding is one `(B-View)`'s materialise clause
            // names: this arm VIEWS a container that is disturbed while the binding is live, so
            // this path needs a store of its own — `(O-Complete)`, per binding and per PATH.
            // An owned temp is how it gets one: `__lift_N = h.inner` carrying no deps is the
            // plain projection bind the F1 materialise already copies on both backends, so
            // nothing here re-derives a copy, which is this function's whole contract.
            //
            // Gated on the walk's answer and not on the shape, because a projection arm whose
            // container is NEVER disturbed must keep aliasing — that is what `(B-View)` is for,
            // and copying it would lose a write that lands today.  Only the ARM is rewritten;
            // an arm that mints keeps its own store and the join reconciles them as before.
            Value::Call(d, _)
                if crate::use_analysis::is_projection_op(data, *d)
                    && bound != u16::MAX
                    && self.views_to_materialise.contains_key(&bound) =>
            {
                *materialised = true;
                let (base, opt) = function.tp(bound).peel_optional();
                match base {
                    Type::Reference(r, _) => Some(ArmBind::Bind(Self::reopt(
                        opt,
                        Type::Reference(*r, Deps::none()),
                    ))),
                    Type::Enum(r, true, _) => Some(ArmBind::Bind(Self::reopt(
                        opt,
                        Type::Enum(*r, true, Deps::none()),
                    ))),
                    // A COLLECTION arm needs the copy EMITTED, not just a temp bound: the
                    // vector bind's copy-vs-view is decided at PARSE time, so a temp typed
                    // without deps arrives too late to be heard.  Same buffer-and-refill the
                    // whole-vector copy takes (loft#1377's arm), reached here because the walk
                    // NAMES a collection view on this tree — which is what made the same idea
                    // inert where it does not (loft#1399).
                    Type::Vector(inner, _) => {
                        let wrapper = format!("main_vector<{}>", inner.name(data));
                        if data.name_type(&wrapper, data.def(self.d_nr).source) == u16::MAX {
                            return None;
                        }
                        let elem = data.vector_element_type(inner, self.database)?;
                        Some(ArmBind::CopyVector {
                            tp: Type::Vector(inner.clone(), Deps::none()),
                            elem: i32::from(elem),
                        })
                    }
                    _ => None,
                }
            }
            Value::Call(d, _) => {
                let def = data.def(*d);
                if !def.is_loft_defined() {
                    return None;
                }
                let (returned, opt) = def.returned().peel_optional();
                let tp = match returned {
                    Type::Reference(r, _) => Type::Reference(*r, Deps::none()),
                    Type::Enum(r, true, _) => Type::Enum(*r, true, Deps::none()),
                    _ => return None,
                };
                match crate::use_analysis::ownership_of(data, self.d_nr, tail) {
                    // @PLN155 phase 2 — `None` means "no bind needed", the answer for a tail
                    // that owns.  Declining on `Unknown` would BIND a temp on a shape the
                    // oracle cannot read, which adds a store rather than withholding a free;
                    // the safe direction here is not obviously the conservative one.
                    crate::use_analysis::Own::Owned | crate::use_analysis::Own::Unknown => None,
                    crate::use_analysis::Own::Borrowed { .. }
                    | crate::use_analysis::Own::Join { .. } => {
                        Some(ArmBind::Bind(Self::reopt(opt, tp)))
                    }
                }
            }
            Value::Var(x) => {
                // A branch consumed as a call ARGUMENT binds nothing: the argument ALIASES
                // the caller's variable (calls.md F-ParamHeap), and a callee that hands its
                // argument back must hand back that variable's store, not a temp's.  Copying
                // the arm there gave D-own-16's `c = maybe_b(c ?? M {}, i)` a temp that died
                // at the statement while `c` still named it.
                if bound == u16::MAX || *x == bound {
                    return None;
                }
                // A compiler temp is not a plain variable a bind may copy: a `__lift_N` is
                // this rewrite's own product and an `_elm_N` is a slot inside a container.
                // The ONE exception is a null-discharge HOIST on a binding the walk has named
                // as a view to materialise.  There the arm is the subject PROJECTION wearing
                // the temp the lowering hoisted it into — the arm right above copies that same
                // projection when it is spelled inline — and `(H-Materialise)` gives the path
                // a store of its own, which is what makes `c = v[1] ?? d` agree with `c = v[1]`
                // across a disturbance of `v` (loft#1401).
                //
                // Gated on the walk's answer and not on the shape, for the reason the
                // projection arm states: a discharge whose container is NEVER disturbed must
                // keep aliasing, and copying it would lose a write that lands today.
                //
                // A hoist that holds a WHOLE local is the other exception, and it needs no walk:
                // `(e as B?) ?? d`, `(if c { s } else { null }) ?? d` hand the binding the local
                // itself, which `(B-Copy)` copies wherever it is spelled — the temp only saved a
                // second evaluation of the subject (loft#1752, `whole_value_hoists_in`).
                if function.is_compiler_generated(*x) && !self.whole_value_hoists.contains(x) {
                    if !(is_discharge_hoist(function, *x)
                        && self.views_to_materialise.contains_key(&bound))
                    {
                        return None;
                    }
                    *materialised = true;
                }
                // Only for a binding the join is the ONE assignment of.  A binding assigned
                // elsewhere as an owner — first bound by a plain copy and re-bound from the
                // join, `r = x; for … { r = v[i] ?? x }` — takes the runtime join bind
                // (`OpBindOrCopy` in codegen), which copies whatever the join hands it; lifting
                // there would turn one binding's fact into a borrow at every one of its Sets
                // and orphan the copies the others made (`@FR-O-Latest`: the fact belongs to
                // the assignment, and a type-level list cannot carry two).
                //
                // Unless the walk has NAMED this binding, which is a fact about THIS
                // assignment and not about the type: the arm needs a store of its own or the
                // binding aliases a container that has been disturbed under it.  Nothing
                // type-level is claimed either way — `lift_join_arm_tails` skips the dep
                // rewrite for a multi-assigned binding on its own, so the copy lands and the
                // other Sets keep the fact they had.  The PROJECTION arm above never asked
                // this question, so a `c = Box{…}; c = v[1] ?? Box{…}` differed from its
                // inline twin only in which of the two arms the projection was spelled in
                // (loft#1401).
                // A whole-value hoist bound into a binding the parser typed as a VIEW is the
                // same kind of fact: no runtime join bind copies into a borrow, so without the
                // lift `t = S{…}; t = (if c { s } else { null }) ?? d` aliased `s` (loft#1752).
                // An owner-typed binding keeps the runtime copy it already gets.
                // @FR-O-Proxy asks copy — chooses whether this arm copies; authorises no free.
                let whole_into_view = (self.whole_value_hoists.contains(x)
                    || !function.is_compiler_generated(*x))
                    && !function.tp(bound).depend().is_empty();
                if self.multi_assigned.contains(&bound)
                    && !self.views_to_materialise.contains_key(&bound)
                    && !whole_into_view
                {
                    return None;
                }
                let (base, opt) = function.tp(*x).peel_optional();
                match base {
                    Type::Reference(r, _) => Some(ArmBind::Bind(Self::reopt(
                        opt,
                        Type::Reference(*r, Deps::none()),
                    ))),
                    // A struct-enum is the same heap record shape (`Type::heap_def_nr`), and
                    // codegen copies its `Var` bind exactly as a struct's.
                    Type::Enum(r, true, _) => Some(ArmBind::Bind(Self::reopt(
                        opt,
                        Type::Enum(*r, true, Deps::none()),
                    ))),
                    Type::Vector(inner, _) => {
                        // The buffer's function-entry allocation names the wrapper type by
                        // name; a vector kind this program never built has none, and the
                        // refill op names the element type only the registry knows.
                        let wrapper = format!("main_vector<{}>", inner.name(data));
                        if data.name_type(&wrapper, data.def(self.d_nr).source) == u16::MAX {
                            return None;
                        }
                        let elem = data.vector_element_type(inner, self.database)?;
                        Some(ArmBind::CopyVector {
                            tp: Type::Vector(inner.clone(), Deps::none()),
                            elem: i32::from(elem),
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The type a per-arm temp takes for this fn-ref call, or `None` where the arm keeps the
    /// value it has: the target must resolve and the fn-ref must capture no store the oracle
    /// cannot name.  What the temp is typed as is what the SINGLE bind of that call would leave
    /// the binding holding — `@FR-O-Complete` asks for the fact per binding, per path, and the
    /// temp is that binding:
    ///
    ///   * an OWNED return — the closure minted the store — is adopted, so the temp owns;
    ///   * a BORROWED return is what `@FR-O-Move` says the caller COPIES: a record's bind does
    ///     that in codegen (`callee_of`, the same arm a direct call takes), and a collection
    ///     is copied by the CALLEE into its per-call buffer — so both own.  A collection the
    ///     callee hands back as a raw view (`returns_borrowed_view`, a keyed field or an index
    ///     read, which the delivery does not copy) is the one shape left alone: the join then
    ///     borrows the caller's store on that arm, which is the view it always was;
    ///   * a `Join` return is the runtime question: a record temp owns either way through the
    ///     bound spelling's `OpBindOrCopy`, and a collection temp carries the base as its dep,
    ///     which is what `scan_set` reads to free it by identity.
    ///
    /// The fallback `None` is *"this arm hands the join a value the join may keep borrowing"*:
    /// an unresolved target or a blocked capture answers as before (the arm's store leaks
    /// rather than being freed under a caller), and a view stays a view.
    fn arm_callref_lift_type(&self, val: &Value, data: &Data, function: &Function) -> Option<Type> {
        let Value::CallRef(v_nr, _) = val.unspan() else {
            return None;
        };
        let d_nr = match self.fnref_target.get(v_nr).copied() {
            Some(d) if d != u32::MAX => d,
            _ => return None,
        };
        let def = data.def(d_nr);
        if def.code == Value::Null
            || crate::use_analysis::callref_capture_blocks(data, self.d_nr, val)
        {
            return None;
        }
        let _ = function;
        let (returned, opt) = def.returned().peel_optional();
        let record = matches!(returned, Type::Reference(_, _) | Type::Enum(_, true, _));
        let witness = match crate::use_analysis::ownership_of(data, self.d_nr, val) {
            crate::use_analysis::Own::Join { base } => {
                if base == u16::MAX {
                    return None;
                }
                if record {
                    Deps::none()
                } else {
                    Deps::frame1(base)
                }
            }
            // @PLN155 phase 2 — **this is the site the split was built for.**  The comment
            // below describes hand-compensating for a fallback that is now its own verdict:
            // `Unknown` IS "the `CallRef` arm could not name a base", so this arm no longer
            // has to infer that from `Owned` and ask the callee to find out.  Kept joined for
            // now because separating them CHANGES what is emitted, and that is phase 2b with
            // its own measurement — but this is the first reader to separate.
            crate::use_analysis::Own::Unknown if crate::keys::own_declines("witness") => {
                // @PLN155 phase 2b — DECLINE, and **it is REFUTED.**  The reasoning was that
                // the arm below infers "the summary could not name a base" from an `Owned` it
                // does not trust and asks the callee to find out, so `Unknown` saying it
                // outright makes the inference unnecessary.  Measured: it is a WRONG VALUE on
                // both backends (guard 1335 — `rr.x` reads 99 where the mapper must give 81),
                // plus guard 1323 on the interpreter.  The arm's hand-compensation is not a
                // workaround for the fail-open, it is the mechanism that makes this site
                // right, and declining removes it.  Phase 2a called this "the first reader to
                // separate"; the measurement says otherwise, which is why it was measured.
                return None;
            }
            crate::use_analysis::Own::Owned | crate::use_analysis::Own::Unknown => {
                // ⚠ `Own::Owned` is ALSO the `CallRef` arm's fallback for a base it cannot
                // name — its own doc says so — so at a site that frees it is not a verdict.
                // A second hop reaches it: `fwd = fn(q) { inner(q) }` resolves (loft#1329
                // made a fn-ref capture resolvable) and answers `Owned` because the base
                // arrives through the capture, while the callee's own type says the return
                // borrows `q`.  Taking that at face value gave the temp an unwitnessed free
                // of the CALLER's collection, one per evaluation.
                //
                // So ask the CALLEE, which is where the fact is: a return that borrows a
                // visible parameter is not owned, whatever the caller-side walk resolved.
                // Its DECLARED dep still names WHICH parameter, so the arm keeps a binding
                // and the store is decided per execution by identity against the argument —
                // the borrow arm hands back that store and declines, the mint arm is
                // distinct and frees.  With no nameable argument there is no witness, and
                // the arm keeps the leak it had rather than freeing blind.
                if def.returns_borrowed_view() {
                    let base =
                        crate::use_analysis::callref_declared_borrow_base(data, self.d_nr, val)?;
                    if self.multi_assigned.contains(&base) {
                        return None;
                    }
                    if record {
                        Deps::none()
                    } else {
                        Deps::frame1(base)
                    }
                } else {
                    Deps::none()
                }
            }
            crate::use_analysis::Own::Borrowed { .. } => {
                if !record && def.returns_borrowed_view() {
                    return None;
                }
                Deps::none()
            }
        };
        let tp = match returned {
            Type::Reference(d, _) => Type::Reference(*d, Deps::none()),
            Type::Enum(d, true, _) => Type::Enum(*d, true, Deps::none()),
            Type::Vector(inner, _) => Type::Vector(inner.clone(), witness),
            Type::Hash(d, k, _) => Type::Hash(*d, k.clone(), witness),
            Type::Sorted(d, k, _) => Type::Sorted(*d, k.clone(), witness),
            Type::Index(d, k, _) => Type::Index(*d, k.clone(), witness),
            Type::Radix(d, k, _) => Type::Radix(*d, k.clone(), witness),
            Type::Trie(d, k, _) => Type::Trie(*d, k.clone(), witness),
            _ => return None,
        };
        Some(Self::reopt(opt, tp))
    }
}
