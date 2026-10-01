// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! The statements of a block or loop: `convert` places the releases and re-mints each statement
//! owes before and after it runs.

use super::backings::literal_backing_of;
use super::branches::needs_pre_init;
use super::drops::drop_hook;
use super::handoff::{copy_hands_off, copy_moves_drop_from, drop_handoff_node};
use super::{Scopes, call};
use crate::data::{Block, Data, Type, Value, v_set};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;

/// Is `stmt` the re-mint of a vector literal's backing — `OpDatabase(__vdb_N, …)` — which the
/// literal's following statements refill?
fn remints_literal_backing(stmt: &Value, function: &Function, data: &Data) -> bool {
    matches!(stmt.unspan(), Value::Call(d, args) if *d == data.def_nr("OpDatabase")
        && matches!(args.first().map(Value::unspan), Some(Value::Var(b))
            if (*b as usize) < function.count() as usize && function.name(*b).starts_with("__vdb_")))
}

/// [`walk_unconditional`] inside a tuple's member-write block, which also enters an `if` whose
/// test reads one of the block's member `stash`es: that `if` is a nullable member's own null
/// test, and the path it skips holds no record, so a copy in its arm runs on every path that
/// has one to release.
fn walk_member_writes(v: &Value, stash: &HashMap<u16, (u16, u16)>, f: &mut impl FnMut(&Value)) {
    f(v);
    match v.unspan() {
        Value::If(test, then, els) => {
            walk_member_writes(test, stash, f);
            if stash.keys().any(|t| test.reads_var(*t)) {
                walk_member_writes(then, stash, f);
                walk_member_writes(els, stash, f);
            }
        }
        Value::Loop(_) => {}
        other => other.for_each_child(&mut |c| walk_member_writes(c, stash, f)),
    }
}

/// Visit every node of `v` that runs whenever `v` does: not an `if`'s arms and not a loop's body.
fn walk_unconditional(v: &Value, f: &mut impl FnMut(&Value)) {
    f(v);
    match v.unspan() {
        Value::If(test, _, _) => walk_unconditional(test, f),
        Value::Loop(_) => {}
        other => other.for_each_child(&mut |c| walk_unconditional(c, f)),
    }
}

/// The call buffer a whole-value copy of a tuple MEMBER hands the release of, where that member
/// was minted by its own initializing call (`t = (mk(11), 1)`), or `None`.
///
/// `(H-Move)`: `u = t` of a tuple the function built moves `t`, member by member (loft#1361
/// lowers it onto one copy per member).  A member built by a literal names its work-ref in the
/// tuple's type and [`drop_bearing_source`] finds it; a member a CALL minted names nothing
/// there, because its store is frame-owned, and its pairing lives only in the scan's
/// `tuple_call_mint`.  Without this the copy moved nothing, and both the copy and the member
/// ran the hook (loft#1563).
fn call_minted_member_handoff(
    (base, idx): (u16, u16),
    dest: &Value,
    function: &Function,
    data: &Data,
    call_mints: &HashMap<u16, HashMap<u16, Option<u16>>>,
) -> Option<u16> {
    let buf = (*call_mints.get(&base)?.get(&idx)?)?;
    match dest.unspan() {
        Value::Var(target) if function.name(*target).starts_with("__ref") => {
            copy_moves_drop_from(function, data, *target, buf, true)
        }
        // A PLACE whose container releases what it holds — the return record's field a
        // returned tuple is copied into (`synthetic_tuple_return`) — takes the release too.
        dest if copy_hands_off(dest, function, data) => Some(buf),
        _ => None,
    }
}

fn contains_loop(node: &Value) -> bool {
    if matches!(node.unspan(), Value::Loop(_)) {
        return true;
    }
    let mut found = false;
    node.for_each_child(&mut |c| {
        if !found && contains_loop(c) {
            found = true;
        }
    });
    found
}

/// Every variable assigned inside a LOOP BODY reachable from `node` — the `Loop`-recursing
/// twin of `find_first_ref_vars`, which deliberately does not descend into a loop (loft#1156).
///
/// ⚠ It descends to the `Value::Loop` FIRST and only then collects.  Walking the whole
/// statement instead collects the statement's own top-level `Set` — and a statement whose
/// value happens to CONTAIN a loop (`test_value = { … for … { } … }`, the shape `expr!`
/// wraps every snippet in) then hoisted the destination of the assignment being scanned.
fn collect_loop_body_sets(node: &Value, mapping: &HashMap<u16, u16>, out: &mut Vec<u16>) {
    if let Value::Loop(b) = node.unspan() {
        for op in &b.operators {
            collect_sets_in(op, mapping, out);
        }
        return;
    }
    node.for_each_child(&mut |c| collect_loop_body_sets(c, mapping, out));
}

/// Every variable assigned at any depth inside `node`.
fn collect_sets_in(node: &Value, mapping: &HashMap<u16, u16>, out: &mut Vec<u16>) {
    if let Value::Set(v, _) = node.unspan() {
        let resolved = *mapping.get(v).unwrap_or(v);
        if !out.contains(&resolved) {
            out.push(resolved);
        }
    }
    node.for_each_child(&mut |c| collect_sets_in(c, mapping, out));
}

/// Whether the first use of a variable in execution order READS it or WRITES it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FirstUse {
    Read,
    Write,
}

/// The first use of `v` in `node`, in execution order.
///
/// This is the liveness question [`Scopes::locals_read_after`] asks: a later region that
/// ASSIGNS `v` before reading it is a fresh binding that happens to share a name, not a reader
/// of the value the loop produced.  [`mentions_var`] cannot tell those apart — it answers
/// "does `v` appear here", so an independent binding reads as a use of the loop's value.
///
/// Two orderings carry the meaning.  A `Set`'s VALUE is evaluated before its target is
/// written, so `r = f(r)` reads first.  A `Loop` body may run ZERO times, so a write inside it
/// never kills the binding — only a read there establishes liveness.
fn first_use_of(node: &Value, v: u16) -> Option<FirstUse> {
    match node.unspan() {
        Value::Set(t, val) => first_use_of(val, v).or_else(|| (*t == v).then_some(FirstUse::Write)),
        Value::Var(x) if *x == v => Some(FirstUse::Read),
        Value::Block(bl) => first_use_in_seq(&bl.operators, v),
        Value::Insert(ops) => first_use_in_seq(ops, v),
        Value::Loop(b) => match first_use_in_seq(&b.operators, v) {
            Some(FirstUse::Read) => Some(FirstUse::Read),
            _ => None,
        },
        Value::If(test, t, f) => first_use_of(test, v).or_else(|| {
            match (first_use_of(t, v), first_use_of(f, v)) {
                // A read on EITHER path makes the binding live; only a write on BOTH kills it.
                (Some(FirstUse::Read), _) | (_, Some(FirstUse::Read)) => Some(FirstUse::Read),
                (Some(FirstUse::Write), Some(FirstUse::Write)) => Some(FirstUse::Write),
                _ => None,
            }
        }),
        // The IR nodes that name a variable as a bare `u16` rather than through `Value::Var`.
        // Each one READS the variable it names (a `TuplePut` writes an ELEMENT, so the tuple
        // itself must already hold a value), and reading them as such is also the safe
        // direction: it can only keep a hoist that loft#1156 wants, never drop one.
        Value::TupleGet(x, _) if *x == v => Some(FirstUse::Read),
        Value::TuplePut(x, _, inner) => {
            first_use_of(inner, v).or_else(|| (*x == v).then_some(FirstUse::Read))
        }
        Value::FnRefDnr(x) if *x == v => Some(FirstUse::Read),
        Value::FnRef(_, clos_var, _) if *clos_var == v => Some(FirstUse::Read),
        _ => {
            let mut found = None;
            node.for_each_child(&mut |c| {
                if found.is_none() {
                    found = first_use_of(c, v);
                }
            });
            found
        }
    }
}

/// The first use of `v` across `ops`, evaluated in order.
fn first_use_in_seq(ops: &[Value], v: u16) -> Option<FirstUse> {
    ops.iter().find_map(|op| first_use_of(op, v))
}

/// The literal-backing accumulator a statement REPOINTS at a destination, if it has one.
///
/// A keyed or vector literal builds through a function-scoped accumulator (`__kvb_N` /
/// `__vdb_N`, [`crate::variables::owns_literal_backing_store`]) that normally OWNS the store it
/// builds into — which is why the scope-exit sweep frees it.  Where the destination is a
/// CAPTURE, @PLN93's build-into-target lowering instead REPOINTS the accumulator at that
/// destination, and the sweep then frees a store belonging to the frame that built the closure
/// (loft#1331).
///
/// The repoint is what this recognises, and it is the exact discriminator: a value-position
/// literal's block only ever assigns the accumulator `null` — it allocates and builds into a
/// store of its own — while a repointing block assigns it the destination first.  A plain
/// non-captured local rebind mints no accumulator at all.  Returning the accumulator here is
/// therefore the same question as *"does this statement leave it naming a store this frame does
/// not own?"*.
///
/// One home for every lowering: the keyed replace reaches the sweep as a bare `Block`, the
/// empty-literal clear as one wrapped in `OpReplaceKeyed`, and both are the same defect.
fn repointed_literal_accumulator(node: &Value, function: &Function) -> Option<u16> {
    if let Value::Block(bl) = node.unspan()
        && let Some(Value::Var(acc)) = bl.operators.last().map(Value::unspan)
        && crate::variables::owns_literal_backing_store(function.name(*acc))
        && bl.operators.iter().any(|op| {
            matches!(op.unspan(), Value::Set(t, val)
                if t == acc && !matches!(val.unspan(), Value::Null))
        })
    {
        return Some(*acc);
    }
    let mut found = None;
    node.for_each_child(&mut |c| {
        if found.is_none() {
            found = repointed_literal_accumulator(c, function);
        }
    });
    found
}

impl Scopes<'_> {
    /// The per-path flag of `var` (loft#1515): `false` at function entry, set where a copy that
    /// stops `var` runs, read at `var`'s scope-end release and at a rebind of it.  One per
    /// variable however many copies share it — a source handed out by one arm and a destination
    /// filled off a parameter by another ask the same question of the same record — so a second
    /// request returns the first.
    /// The per-path flag of tuple member `(t, idx)` ([`Self::tuple_moved`]), minted on first use.
    fn tuple_moved_flag(&mut self, function: &mut Function, (t, idx): (u16, u16)) -> u16 {
        if let Some(&flag) = self.tuple_moved.get(&(t, idx)) {
            return flag;
        }
        let name = format!("__tmov_{}#{t}_{idx}", function.name(t));
        let flag = function.add_temp_var(&name, &Type::Boolean);
        self.var_scope.insert(flag, 0);
        self.var_order.push(flag);
        self.tuple_moved.insert((t, idx), flag);
        flag
    }

    /// Convert the content of loops and blocks.
    /// `is_return` should be true for the function body block of a non-void
    /// function — frees must happen before the tail expression returns.
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn convert(
        &mut self,
        bl: &Block,
        function: &mut Function,
        data: &Data,
        is_return: bool,
    ) -> Vec<Value> {
        let mut ls = Vec::new();
        // Releases owed at the END of the current statement: a statement's own parts arrive
        // flat, so they wait for the statement boundary, a `Line` marker or the block's end.
        let mut at_end: Vec<Value> = Vec::new();
        self.fnref_bound.push(HashSet::default());
        // `@FR-L-CapKeep` — every holder the frame compares by store starts at null at the
        // function's head, so each later read of one is dominated by a bind (SLOTS.md § the
        // reserve does not initialise), and each later `Set` is a rebind.
        if self.fnref_bound.len() == 1 && self.closure_keep.gated {
            for h in self.closure_keep.holders.clone() {
                if self.var_scope.contains_key(&h) {
                    continue;
                }
                self.register_binding(h, function);
                ls.push(v_set(h, Value::Null));
                if let Some(bound) = self.fnref_bound.last_mut() {
                    bound.insert(h);
                }
            }
        }
        for (i, v) in bl.operators.iter().enumerate() {
            if matches!(v.unspan(), Value::Line(_)) {
                ls.append(&mut at_end);
            }
            // loft#1156 — a local a LOOP BODY first assigns and something AFTER the loop
            // reads.  Scoped to the body block, its store is freed at the end of every
            // iteration and the later read lands on a freed record: measured, an `A` read
            // `B`'s bytes once the slot was recycled, and `LOFT_STRICT_STORES=1` does NOT
            // catch it — the slot is legitimately free, so nothing stands between a user
            // and the wrong number.  Hoisting the SCOPE is the cure rather than moving the
            // free: pre-initialised here the local gets ONE store that each iteration
            // copies into, which is byte-for-byte the IR a hand-written `e: A = A { … }`
            // before the loop already produces.
            //
            // Registered BEFORE the loop is scanned, for the reason `scan_if` gives at its
            // own pre-init: the body's `Set` must see the variable as already assigned and
            // take the reassignment path, not `claim()`.
            let mut hoist: Vec<u16> = Vec::new();
            self.locals_read_after(v, &bl.operators[i + 1..], function, &mut hoist);
            for &h in &hoist {
                self.register_binding(h, function);
                // The pre-init holds nothing, and every later pass of the loop reaches the
                // local's binding holding what the pass before bound — so that binding displaces
                // a record this frame owns.  Recorded as owned for the reason a first `Set`
                // records it (@FR-O-Latest): without it the binding released nothing, and every
                // pass but the last lost its record.  The release is guarded on the record being
                // live, so the first pass, which displaces the null, releases nothing.
                if matches!(
                    function.tp(h).base(),
                    Type::Reference(_, _) | Type::Enum(_, true, _)
                ) {
                    self.owned_refs.insert(h, self.loops.len());
                }
                ls.push(if matches!(function.tp(h), Type::Text(_)) {
                    v_set(h, Value::Text(String::new()))
                } else {
                    v_set(h, Value::Null)
                });
            }
            // `s = S {…}` on a live record local lowers to `OpDatabase(s, tp)` on its
            // existing store, not to a `Set`: a REBUILD, and the record it overwrites is
            // released through its hook exactly as a reassigned one is.  The first
            // construction of a local (outside a loop) displaces nothing.
            let rebuilt = self.in_place_rebuild(v, function, data);
            let rebind_release = self.vector_rebind_release(v, function, data);
            let mut keep_detach = Vec::new();
            if self.closure_keep.gated && !self.loops.is_empty() {
                keep_detach = self.closure_keep_backing(v, function, data, &[]);
                keep_detach.extend(self.closure_keep_detach(v, function, data, &[]));
                if !keep_detach.is_empty() && !self.closure_keep.links.is_empty() {
                    let (link_pre, links, link_post) = self.closure_keep_links(function);
                    keep_detach = self.closure_keep_backing(v, function, data, &links);
                    keep_detach.extend(self.closure_keep_detach(v, function, data, &links));
                    let mut wrapped = link_pre;
                    wrapped.append(&mut keep_detach);
                    wrapped.extend(link_post);
                    keep_detach = wrapped;
                }
            }
            let call_rebind = self
                .vector_call_rebind(v, function, data)
                .or_else(|| self.closure_keep_rebind(v, function, data))
                .or_else(|| self.fnref_call_rebind(v, function, data));
            let promoted_refill = self.promoted_vector_refill(v, function, data);
            // A literal's `Set` heads the statements that fill its new backing, so its release
            // waits for the statement's end; any other rebind is a whole statement already.
            let rebind_is_group = matches!(v.unspan(), Value::Set(_, rhs)
                if literal_backing_of(rhs, function, data).is_some());
            let adopted_detach = self.adopted_backing_detach(v, function, data);
            if let Value::Set(ov, _) = v.unspan()
                && matches!(function.tp(*ov).base(), Type::Function(..))
                && let Some(bound) = self.fnref_bound.last_mut()
            {
                bound.insert(*self.var_mapping.get(ov).unwrap_or(ov));
            }
            let outer_target = self.keep_build_target;
            if let Value::Set(t, rhs) = v.unspan()
                && matches!(rhs.unspan(), Value::Block(b) if b.name == "fn_ref_with_closure")
            {
                self.keep_build_target = Some(*self.var_mapping.get(t).unwrap_or(t));
            }
            let sv = self.scan(v, function, data);
            self.keep_build_target = outer_target;
            let loop_depth = self.loops.len();
            let mut arm_moved: Vec<(u16, u16)> = Vec::new();
            // Arm the hand-offs this statement makes, AFTER it is scanned: its own displaced
            // release and its retirement read the facts of the assignments before it, and what
            // it hands off applies to the value it has just assigned (`@FR-O-Latest`).  Armed
            // before the scan, `x = mk(); x = p` suppressed the release of the displaced `mk()`
            // with the parameter copy's fact, then retired that fact and dropped the copy.
            {
                let Self {
                    drop_transferred,
                    arm_lift_temps,
                    per_path_pairs,
                    tuple_call_mint,
                    tuple_member_now,
                    tuple_depth,
                    var_scope,
                    scope,
                    ..
                } = self;
                v.walk(&mut |n| {
                    drop_handoff_node(
                        n,
                        function,
                        data,
                        drop_transferred,
                        arm_lift_temps,
                        per_path_pairs,
                    );
                });
                // A call-minted tuple member's release moves only where the copy is CERTAIN to
                // run: a copy of a tuple in the tuple's own scope, and not inside an `if` arm or
                // a loop body below this statement.  A copy that only some runs perform would
                // move the release on the path that skips it too, and lose it there; that case
                // keeps the release with both sides (D-heap-15 records it).  Only the whole-tuple
                // bind's copies move (`tuple_member_move`), and a returned tuple's
                // (`synthetic_tuple_return`): `(t.0, 2)` spells a copy of a member
                // of a container, which `(H-Copy-Refuse)` refuses, and it keeps both releases.
                let certain: HashMap<u16, HashMap<u16, Option<u16>>> = tuple_call_mint
                    .iter()
                    .filter(|(t, _)| var_scope.get(t) == Some(scope))
                    .map(|(t, m)| (*t, m.clone()))
                    .collect();
                let copy_d = data.def_nr("OpCopyRecord");
                let append_d = data.def_nr("OpAppendVector");
                // A member whose record has NO claimant (a sole-owner mint: a bufferless call,
                // a generator's advance) has no buffer to mark as handed off.  Its moved
                // release is the bare free: the pairing is dropped, so the member keeps its
                // free and loses its hook, as a buffered member does through the buffer.
                let mut sole_moved: Vec<(u16, u16)> = Vec::new();
                walk_unconditional(v, &mut |n| {
                    let Value::Block(b) = n else { return };
                    // The whole-tuple bind, and a returned tuple: `(H-Move)` moves a variable
                    // this function owns when it is returned, member by member into the
                    // return record.
                    if !matches!(b.name, "tuple_member_move" | "synthetic_tuple_return") {
                        return;
                    }
                    let whole_move = b.name == "tuple_member_move";
                    // A NULLABLE member is written through a stash (`__ref_2 = a.1`) and copied
                    // under its own null test; the stash names the member it holds.
                    let mut stash: HashMap<u16, (u16, u16)> = HashMap::default();
                    n.walk(&mut |c| {
                        if let Value::Set(t, rhs) = c
                            && function.is_compiler_generated(*t)
                            && let Value::TupleGet(base, idx) = rhs.unspan()
                        {
                            stash.insert(*t, (*base, *idx));
                        }
                    });
                    walk_member_writes(n, &stash, &mut |c| {
                        let Value::Call(d, args) = c else { return };
                        let member = match args.first().map(Value::unspan) {
                            Some(Value::TupleGet(base, idx)) => Some((*base, *idx)),
                            Some(Value::Var(t)) => stash.get(t).copied(),
                            _ => None,
                        };
                        // loft#1645 — the same move of a tuple bound OUTSIDE this block, with no
                        // loop between: it runs on some paths only, so it takes a per-path flag.
                        if whole_move
                            && *d == copy_d
                            && args.len() >= 3
                            && let Some(member) = member
                            && !certain.contains_key(&member.0)
                            && tuple_depth.get(&member.0) == Some(&loop_depth)
                            && tuple_call_mint
                                .get(&member.0)
                                .is_some_and(|m| m.contains_key(&member.1))
                            && !arm_moved.contains(&member)
                        {
                            arm_moved.push(member);
                        }
                        if *d == copy_d
                            && args.len() >= 3
                            && let Some(member) = member
                        {
                            if let Some(b) = call_minted_member_handoff(
                                member, &args[1], function, data, &certain,
                            ) {
                                drop_transferred.insert(b);
                            }
                            if certain.get(&member.0).and_then(|m| m.get(&member.1)) == Some(&None)
                                && (matches!(args[1].unspan(),
                                        Value::Var(dst) if function.name(*dst).starts_with("__ref"))
                                    || copy_hands_off(&args[1], function, data))
                            {
                                sole_moved.push(member);
                            }
                        }
                        // A VECTOR member is copied by an append into the copy's own backing, and
                        // its elements' release moves from the member's backing the same way
                        // (loft#1588), read off the assignment the tuple holds now
                        // (`tuple_member_now`) and only where the copy is certain to run.
                        if *d == append_d
                            && let Some(Value::TupleGet(base, idx)) = args.get(1).map(Value::unspan)
                            && var_scope.get(base) == Some(scope)
                            && let Some(&b) = tuple_member_now.get(base).and_then(|m| m.get(idx))
                            && drop_hook(function, b, data).is_some()
                        {
                            drop_transferred.insert(b);
                        }
                    });
                });
                for (base, idx) in sole_moved {
                    if let Some(m) = tuple_call_mint.get_mut(&base) {
                        m.remove(&idx);
                    }
                }
            }
            ls.extend(keep_detach);
            if let Some((pre, _)) = &rebuilt {
                ls.extend(pre.iter().cloned());
            }
            if let Some((pre, _)) = &call_rebind {
                ls.extend(pre.iter().cloned());
            }
            if let Some((pre, _, _)) = &promoted_refill {
                ls.extend(pre.iter().cloned());
            }
            if let Value::Insert(to_insert) = sv {
                for i in to_insert {
                    ls.push(i.clone());
                }
            } else {
                ls.push(sv);
            }
            arm_moved.sort_unstable();
            for member in arm_moved {
                let flag = self.tuple_moved_flag(function, member);
                ls.push(v_set(flag, Value::Boolean(true)));
            }
            if let Some((_, post)) = rebuilt {
                // A vector literal's backing re-minted in place is REFILLED by the statements
                // after this one, so its snapshot is released at the literal's end.
                if remints_literal_backing(v, function, data) {
                    at_end.extend(post);
                } else {
                    ls.extend(post);
                }
            }
            if let Some((_, post, waits)) = promoted_refill {
                if waits {
                    at_end.extend(post);
                } else {
                    ls.extend(post);
                }
            }
            let call_release = call_rebind.map(|(_, post)| post).unwrap_or_default();
            if rebind_is_group {
                at_end.extend(rebind_release);
                at_end.extend(call_release);
            } else {
                ls.extend(rebind_release);
                ls.extend(call_release);
            }
            ls.extend(adopted_detach);
            // loft#1331 — DETACH an accumulator this statement repointed at a destination the
            // frame does not own, so the scope-exit sweep frees nothing instead of freeing the
            // caller's collection.  @FR-O-Latest is the fact: ownership belongs to the LATEST
            // assignment, and the repoint made the accumulator name a capture.  The sentinel
            // makes that true at RUN time — the sweep still emits its free and finds nothing —
            // while the DISPLACEMENT free that releases the accumulator's own store at the
            // repoint is untouched, which is what @FR-O-Override's blanket veto could not do.
            //
            // @FR-O-Detach places it: after the statement, so it follows every read of the
            // accumulator by the value being built through it.  Skipped where the statement is
            // the block's RESULT, which `expr` below pops — a detach there would become the
            // value the block yields.
            if (bl.result == Type::Void || i + 1 < bl.operators.len())
                && let Some(acc) = repointed_literal_accumulator(v, function)
            {
                ls.push(v_set(
                    acc,
                    Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new()),
                ));
            }
        }
        if !at_end.is_empty() {
            if bl.result == Type::Void || ls.is_empty() {
                ls.append(&mut at_end);
            } else {
                let tail = ls.len() - 1;
                ls.splice(tail..tail, at_end);
            }
        }
        let expr = if ls.is_empty() || bl.result == Type::Void {
            Value::Null
        } else {
            ls.pop().unwrap()
        };
        // @PLN85 skip_free-orphan (case a) — free each `__ncc_N` text temp that a
        // NON-TAIL statement consumes IN PLACE, right after that statement.  A
        // `skip_free` text ncc temp (`v[i] ?? ""`) is suppressed from its own
        // scope-exit free because the ncc block's result ALIASES it (freeing at
        // the ncc block would dangle the value the consumer still reads).  But a
        // text consumer (SetText / assignment / append) COPIES the String, so once
        // the consuming statement completes the temp's backing String is dead —
        // never freed on the interpreter → orphan.  The tail expression is left
        // untouched (case b: it IS the returned value, copied by the caller after
        // return, so any in-function free UAFs).  Native drops the String via RAII
        // and treats `OpFreeText` as a no-op, so the added op is interp-only.
        {
            let mut with_frees = Vec::with_capacity(ls.len());
            for stmt in ls.drain(..) {
                // An `if` whose CONDITION consumes the temp (`if (v[i] ?? "") == k { return
                // … }`) cannot take its free after the statement: an arm that returns never
                // reaches it, one orphan per early exit (loft#1357).  Evaluate the condition
                // into a boolean first, free what it consumed, then branch on the boolean.
                let (pos, inner) = match stmt {
                    Value::Span(b) => (Some(b.0.clone()), b.1.clone()),
                    other => (None, other),
                };
                // A `parallel { … }` arm runs on a WORKER over a copy of this frame: the
                // `__work_N` text a formatted argument builds there is the worker's copy,
                // which nothing frees — the frame's own scope-exit `OpFreeText` releases
                // main's (empty) copy.  Each arm frees the work texts it wrote, on the
                // worker, once its call has consumed them (loft#1357).
                if let Value::Parallel(arms) = &inner {
                    let arms: Vec<Value> = arms
                        .iter()
                        .map(|arm| {
                            let mut work: Vec<u16> = Vec::new();
                            arm.walk(&mut |v| {
                                if let Value::Var(w) = v
                                    && function.name(*w).starts_with("__work_")
                                    && matches!(function.tp(*w).base(), Type::Text(_))
                                    && !work.contains(w)
                                {
                                    work.push(*w);
                                }
                            });
                            if work.is_empty() {
                                return arm.clone();
                            }
                            let mut ops = Vec::with_capacity(work.len() + 1);
                            ops.push(arm.clone());
                            for w in work {
                                ops.push(call("OpFreeText", w, data));
                            }
                            Value::Insert(ops)
                        })
                        .collect();
                    let stmt = match pos {
                        Some(p) => Value::Span(Box::new((p, Value::Parallel(arms)))),
                        None => Value::Parallel(arms),
                    };
                    with_frees.push(stmt);
                    continue;
                }
                if let Value::If(cond, then, els) = &inner {
                    let cond_frees = ncc_text_frees(cond, function, data);
                    if !cond_frees.is_empty() {
                        self.ret_temp_counter += 1;
                        let name = format!("__cond_{}", self.ret_temp_counter);
                        let tmp = function.add_temp_var(&name, &Type::Boolean);
                        self.var_scope.insert(tmp, self.scope);
                        self.var_order.push(tmp);
                        with_frees.extend(ncc_text_preinits(cond, function));
                        with_frees.push(v_set(tmp, (**cond).clone()));
                        with_frees.extend(cond_frees);
                        let branch =
                            Value::If(Box::new(Value::Var(tmp)), then.clone(), els.clone());
                        with_frees.push(match pos {
                            Some(p) => Value::Span(Box::new((p, branch))),
                            None => branch,
                        });
                        continue;
                    }
                }
                let stmt = match pos {
                    Some(p) => Value::Span(Box::new((p, inner))),
                    None => inner,
                };
                let frees = ncc_text_frees(&stmt, function, data);
                with_frees.extend(ncc_text_preinits(&stmt, function));
                with_frees.push(stmt);
                with_frees.extend(frees);
            }
            ls = with_frees;
        }
        // Case b's premise — the tail IS the returned value, so its `__ncc_N` temp must
        // outlive the block — holds only when the block YIELDS the text.  A SCALAR tail that
        // consumes the temp (`len(s.name ?? "")`, `t.0 + len(t.1 ?? "")`) copies out the
        // number and leaves the String to nobody: one orphan per call (loft#1357).  Hoist the
        // value first, then free what it consumed, and let the tail be the hoisted scalar.
        let mut expr = expr;
        if !matches!(expr, Value::Null)
            && matches!(
                bl.result,
                Type::Integer(_)
                    | Type::Float
                    | Type::Single
                    | Type::Boolean
                    | Type::Character
                    | Type::Enum(_, false, _)
            )
        {
            let ncc_frees = ncc_text_frees(&expr, function, data);
            if !ncc_frees.is_empty() {
                // An explicit `return <e>` hoists `<e>` and keeps the `return`.
                let (inner, was_return) = match expr.unspan() {
                    Value::Return(i) => ((**i).clone(), true),
                    _ => (expr.clone(), false),
                };
                self.ret_temp_counter += 1;
                let name = format!("__ret_{}", self.ret_temp_counter);
                let tmp = function.add_temp_var(&name, &bl.result);
                self.var_scope.insert(tmp, self.scope);
                self.var_order.push(tmp);
                ls.extend(ncc_text_preinits(&inner, function));
                ls.push(v_set(tmp, inner));
                ls.extend(ncc_frees);
                expr = if was_return {
                    Value::Return(Box::new(Value::Var(tmp)))
                } else {
                    Value::Var(tmp)
                };
            }
        }
        let scope_vars = self.variables(self.scope);
        for &v in &scope_vars {
            self.var_mapping.remove(&v);
        }
        let frees = self.free_vars(is_return, &expr, function, data, &bl.result, self.scope);
        for v in frees {
            ls.push(v);
        }
        // After the block's own frees: a capture the record adopted is released by the
        // record, and the frame's release of it asks whether the record is still live.
        if !is_return {
            let pass_end = self.closure_keep_pass_end(bl, function, data);
            ls.extend(pass_end);
        }
        self.fnref_bound.pop();
        ls
    }

    /// loft#1156 — the locals a LOOP, or a statement BLOCK, first assigns that something AFTER it
    /// READS.
    ///
    /// A body local is scoped to the body block, so `get_free_vars` releases its store at the
    /// end of each iteration.  A read after the loop is then a use-after-free — silent, and
    /// invisible to `LOFT_STRICT_STORES=1` because the slot really is free by then; what the
    /// read returns is whatever the allocator handed that slot next.  `--native` refuses the
    /// program instead (`E0425`), which is the same decision made visible: the free analysis
    /// already put the local's death at the block's end and native additionally scopes the
    /// Rust `let` there.  One decision, expressed twice, half of it visible.
    ///
    /// ⚠ **Only a local READ AFTER the loop is taken**, and the exclusions are what keep this
    /// from re-opening loft#1135.  A loop's own VARIABLE is read after the loop routinely
    /// (`LOFT.md` documents it) and must NOT be hoisted: its header assigns it
    /// unconditionally every iteration, and reserving a slot for it at the enclosing scope
    /// registers it in a scope it does not live in — one orphaned store per program.
    /// `was_loop_var` is the declared home for that question.  A local used INSIDE the loop
    /// alone is correctly per-iteration and is left exactly as it is.
    ///
    /// A statement BLOCK is the same decision with one death instead of one per iteration.
    /// Every `match` lowers its arms inside a block of its own (the subject binding, then the
    /// arm chain), a scope the author never wrote: `match e { A => { t = P {…} }, … }; t.id`
    /// freed `t` at that block's end and read it after — a freed store on the interpreter,
    /// `E0425` on native — while the `if` spelling of the same arms declares `t` where it is
    /// read (`scan_if`'s pre-init).
    fn locals_read_after(
        &self,
        op: &Value,
        rest: &[Value],
        function: &Function,
        out: &mut Vec<u16>,
    ) {
        if rest.is_empty() {
            return;
        }
        let mut assigned: Vec<u16> = Vec::new();
        if contains_loop(op) {
            collect_loop_body_sets(op, &self.var_mapping, &mut assigned);
        }
        if let Value::Block(bl) = op.unspan() {
            for inner in &bl.operators {
                collect_sets_in(inner, &self.var_mapping, &mut assigned);
            }
        }
        for v in assigned {
            if self.var_scope.contains_key(&v)
                || out.contains(&v)
                || function.was_loop_var(v)
                || !needs_pre_init(function.tp(v))
            {
                continue;
            }
            // Same guard as `find_first_ref_vars`: a BORROWED type may only be pre-inited
            // once every dep is in scope, or the `OpCreateStack` emitted here reads an
            // uninitialised slot.
            if !function
                .tp(v)
                .depend()
                .iter()
                .all(|d| self.var_scope.contains_key(d))
            {
                continue;
            }
            // LIVE after the loop, not merely mentioned: a later region that assigns `v`
            // before reading it is an independent binding sharing the name, and hoisting
            // those together gives one function-scope variable whose ownership fact is the
            // JOIN of both — which frees a borrow as if it were owned (loft#1332).
            if first_use_in_seq(rest, v) == Some(FirstUse::Read) {
                out.push(v);
            }
        }
    }
}

/// @PLN85 skip_free-orphan (case a): collect the `skip_free` text `__ncc_N` temps
/// whose null-coalesce value-block is nested (as a sub-expression) inside `node` —
/// i.e. the temps this statement CONSUMES IN PLACE.  Descends through expression
/// constructs (`Call`/`If`/`Insert`/…) and INTO `ncc`-named value-blocks (to reach
/// a nested `??`), but STOPS at any other `Block`/`Loop`: those run their own
/// `convert` and free their own temps, so descending would double-free.  A bare
/// `Set(__ncc, …)` outside an `ncc` block (the temp's own declaration inside the
/// ncc block) is deliberately NOT matched — only the value-block's presence counts,
/// which is why the ncc block's own `convert` attributes no free (its statement is
/// the declaration, not a nested ncc consumer).
/// The `OpFreeText` releases a node's CONSUMED `__ncc_` text temps need, ready to splice in.
///
/// One home for `collect_consumed_ncc_text` plus the loop that turns its answer into frees —
/// `Scopes::convert` wrote that pair out three times, once per statement shape it hoists (an
/// `if` condition, a plain statement, a block tail), and each copy had to be found and taught
/// anything the others learned.  The list is a `Vec<Value>` rather than a push-into-a-buffer,
/// because two of the three callers test whether there is anything to free BEFORE deciding to
/// build a temp at all, and emptiness of the frees is that test.
fn ncc_text_frees(node: &Value, function: &Function, data: &Data) -> Vec<Value> {
    let mut out = Vec::new();
    collect_consumed_ncc_text(node, function, false, &mut out);
    out.into_iter()
        .map(|(v, _)| call("OpFreeText", v, data))
        .collect()
}

/// The `__ncc_N = ""` initialisations a statement owes before it, for the `??` temps
/// [`ncc_text_frees`] will free AFTER it but that the statement assigns only inside one arm of
/// a conditional (`n > 0 && f(v[n - 1] ?? "")`, `||`, an `if` or `match` expression — all an
/// `If` here).  The free after the statement runs on every path, so the slot must hold a text
/// on every path: on the path that skips the arm it held whatever an earlier frame left there,
/// and the interpreter freed that (loft#1773, heap-state dependent; native declares the temp
/// initialised).  The init is the same `__ncc_N = ""` a right-nested `??` pre-declares, and
/// the arm's own assignment then overwrites it.
fn ncc_text_preinits(node: &Value, function: &Function) -> Vec<Value> {
    let mut out = Vec::new();
    collect_consumed_ncc_text(node, function, false, &mut out);
    out.into_iter()
        .filter(|&(_, under_arm)| under_arm)
        .map(|(v, _)| v_set(v, Value::Text(String::new())))
        .collect()
}

fn collect_consumed_ncc_text(
    node: &Value,
    function: &Function,
    under_arm: bool,
    out: &mut Vec<(u16, bool)>,
) {
    match node {
        Value::Span(b) => collect_consumed_ncc_text(&b.1, function, under_arm, out),
        // The condition runs on every path through the `If`; each arm on only some.
        Value::If(c, t, e) => {
            collect_consumed_ncc_text(c, function, under_arm, out);
            collect_consumed_ncc_text(t, function, true, out);
            collect_consumed_ncc_text(e, function, true, out);
        }
        Value::Block(bl) if bl.name == "ncc" => {
            for op in &bl.operators {
                if let Value::Set(v, val) = op.unspan()
                    && function.is_staged_text_temp(*v)
                    && function.name(*v).starts_with("__ncc_")
                    // Only the REAL coalesce-subject assignment (a Call / field
                    // access / nested block — a producer of an owned String)
                    // gets an in-place free.  A right-nested `??` (`a ?? (b ?? c)`)
                    // hoists a merge-var pre-declaration `__ncc_N = ""` (a literal
                    // Text init) into the OUTER ncc block while the real
                    // assignment lives in the inner block; collecting the literal
                    // init too freed the temp twice (156 sibling: right-nested
                    // `??` double-free).  A subject is never a bare literal.
                    && !matches!(val.unspan(), Value::Text(_) | Value::Null)
                {
                    out.push((*v, under_arm));
                }
            }
            // Do NOT recurse INTO this ncc block: a nested `??` (`a ?? b ?? c`)
            // lowers to an ncc block whose Set value is ANOTHER ncc block, and
            // that inner block gets its OWN `convert` (and thus its own in-place
            // free pass) when it is scanned.  Recursing here would ALSO collect
            // the inner block's `__ncc_*` temp from the outer level, freeing it
            // twice (a `text.rs:334` double-free on the interpreter for a chained
            // `??` whose first operand is an owned/call-produced text; 156).
            // Non-ncc structures (call args, if-branches) are still descended
            // through by the `_` arm below, so sibling / nested-in-expression ncc
            // blocks are reached exactly once.
        }
        Value::Block(_) | Value::Loop(_) => {}
        _ => node.for_each_child(&mut |c| collect_consumed_ncc_text(c, function, under_arm, out)),
    }
}
