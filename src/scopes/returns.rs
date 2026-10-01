// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! What a RETURN hands out: the shape of a function's return type and value, the locals a return
//! delivers or copies, and a join-return's hooks moved into its arms.

use super::Scopes;
use super::drops::drop_hook;
use super::insert_free::scope_free_op_var;
use crate::data::{Data, Type, Value};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;

/// Whether a function's return type holds a plain value (no heap ownership).
/// Used by the B5-L3 fix in `free_vars` to decide whether saving the tail
/// expression into a `__ret_N` temp is safe.  Heap-owned types are excluded
/// for now — their ownership transfer interacts with `OpFreeRef` emission
/// and needs a separate design pass.
pub(super) fn is_value_return_type(tp: &Type) -> bool {
    // @PLN25: peel `Optional` — a `float?`/`integer?`/… return is the same
    // value-return shape (native reps it as the sentinel-carrying scalar).
    // Without the peel a `-> τ?` tail call with pending free-ops fell through
    // the B5-L3 wrap: the call was emitted as a DISCARDED statement + a
    // fabricated `return null`, which native materialised as `return 0.0`
    // — a silent NON-NULL corruption (the routing elevation-kernel bug).
    // loft#816 — a FLAT pure-value tuple is a value return too.  @PLN135 gave
    // `-> (float, integer)` Rust's own tuple ABI (no synthetic `__tuple`
    // record), and that shape reaches here as a bare `Type::Tuple`; unnamed, an
    // anonymous tail tuple with pending frees fell through to the discard +
    // fabricated `Return(Null)` path and native answered the ZERO initialiser
    // (`return (0.0_f64, 0)`) for the whole tuple.  Silent, and on one backend
    // only — the interpreter read the elements off eval-stack top.
    //
    // Recursing per element is what keeps the bound exact.  A NESTED pure-value
    // tuple qualifies too (loft#817): the wrap emits `Set(__ret_N, <tuple>)` and
    // then reads the temp back, and both halves are nested-aware —
    // `emit_tuple_var_pop_put` writes the leaves and `generate_var` delegates to
    // `emit_tuple_var_push_recursive` to read them.  An element with a lifetime
    // (text / vector / record) is either rewritten to the boxed `Reference`
    // shape upstream or belongs to the @P329 tuple-of-text branch, and neither
    // may be hoisted by a plain `Set` — so those stop the recursion here.
    if let Type::Tuple(elems) = tp.base() {
        return !elems.is_empty() && elems.iter().all(is_value_return_type);
    }
    is_scalar_value_type(tp)
}

/// The scalar half of [`is_value_return_type`]: a type returned in a register,
/// owning nothing on the heap.  Split out so the tuple case reads as "every
/// element is itself a value return", with this as the recursion's base.
fn is_scalar_value_type(tp: &Type) -> bool {
    matches!(
        tp.base(),
        Type::Integer(_)
            | Type::Float
            | Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Enum(_, false, _)
    )
}

/// loft#754 — a return type delivered as a store POINTER (`DbRef`): a vector, a
/// record, or a struct-enum, including one reached through a `&`-parameter
/// place ref.  Names the half of the return space that neither
/// `is_value_return_type` nor the text branch covers, so a tail expression of
/// this shape is hoisted before the scope frees instead of being dropped.
pub(super) fn is_heap_return_type(tp: &Type) -> bool {
    match tp.base() {
        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => true,
        Type::RefVar(inner) => is_heap_return_type(inner),
        _ => false,
    }
}

/// A branch whose TAIL is literally null — `Value::Null` or the
/// `OpNullRefSentinel()` fall-through `parse_match` injects.  Deliberately does
/// NOT recurse into `If`: a branch that merely CONTAINS a null sub-arm is not a
/// null terminal (unifying through it would lose the other sub-arm's value).
impl Scopes<'_> {
    /// The tail of `expr` is an `If` (a lowered if/match) — its arms deliver
    /// into the assignment target via the arm-unification machinery.
    pub(super) fn tail_is_branch(expr: &Value) -> bool {
        match expr {
            Value::If(_, _, _) => true,
            Value::Block(bl) => bl.operators.last().is_some_and(Self::tail_is_branch),
            Value::Insert(ops) => ops.last().is_some_and(Self::tail_is_branch),
            Value::Span(b) => Self::tail_is_branch(&b.1),
            _ => false,
        }
    }
}

/// Does this return expression's terminal value reduce to `null` — a bare `Value::Null`
/// or the reference sentinel `null_nr` names?
///
/// Descends `Block`/`Insert`/`Span` to the tail.  A nested `if` is NOT descended: an arm
/// that IS null and a branch that merely CONTAINS one are different answers, and only the
/// first belongs to a terminal — `return_has_null_arm` is the walker that asks the second.
///
/// A `Return`/`Drop` wrapper IS descended, because this is asked about a return
/// EXPRESSION and the wrapper is the thing being examined.  The `parser::control`
/// siblings (`branch_yields_null`, `arm_yields_direct_null`, `arm_is_null`) ask the same
/// null question about what an ARM hands to a JOIN — where a `return` hands it nothing —
/// and stop at the wrapper.
fn is_null_terminal(expr: &Value, null_nr: u32) -> bool {
    match expr.unspan() {
        Value::Null => true,
        Value::Call(d, _) => *d == null_nr,
        Value::Block(bl) => bl
            .operators
            .last()
            .is_some_and(|o| is_null_terminal(o, null_nr)),
        Value::Insert(ops) => ops.last().is_some_and(|o| is_null_terminal(o, null_nr)),
        Value::Return(inner) | Value::Drop(inner) => is_null_terminal(inner, null_nr),
        _ => false,
    }
}

/// P236 extension (@PLN85 P4-records): `returned_var` with a NULL-arm terminal
/// unifying as a WILDCARD against the other arm's var.  The work-ref null-inits
/// at function entry and a null arm never allocates into it, so
/// `Return(Var(v))` yields the same null the sentinel did — while the PRESENT
/// arm's record now rides the var instead of the freed-TOS channel (the
/// record match/if-arm UAF the poison sweep exposed: the legacy pattern freed
/// the arm's store, then `Return(Null)` handed the caller the freed store's
/// bytes off the eval stack — silently stale without LOFT_POISON, null with).
pub(super) fn returned_var_null_unified(expr: &Value, null_nr: u32) -> u16 {
    match expr {
        Value::Var(v) => *v,
        Value::Block(bl) => {
            let mut v = u16::MAX;
            for o in &bl.operators {
                v = returned_var_null_unified(o, null_nr);
            }
            v
        }
        Value::Return(inner) | Value::Drop(inner) => returned_var_null_unified(inner, null_nr),
        Value::Insert(ops) => ops
            .last()
            .map_or(u16::MAX, |o| returned_var_null_unified(o, null_nr)),
        Value::If(_, t, f) => {
            let t_var = returned_var_null_unified(t, null_nr);
            let f_var = returned_var_null_unified(f, null_nr);
            if t_var == f_var || (f_var == u16::MAX && is_null_terminal(f, null_nr)) {
                t_var
            } else if t_var == u16::MAX && is_null_terminal(t, null_nr) {
                f_var
            } else {
                u16::MAX
            }
        }
        Value::Span(b) => returned_var_null_unified(&b.1, null_nr),
        _ => u16::MAX,
    }
}

pub(super) fn collect_return_sources(expr: &Value, data: &Data, out: &mut Vec<u16>) {
    match expr {
        Value::Var(v) => {
            if !out.contains(v) {
                out.push(*v);
            }
        }
        // @PLN85 P4-records — a block's VALUE is its last op that is NOT a scope-exit
        // free. A captured struct-enum field binds an owned `_mv_<f>` text whose
        // `OpFreeText` is appended AFTER the arm chain (`[Kw { word }, ..] => LetS{…}`),
        // so `.last()` alone hits that free and hides the record sources — the returned
        // enum store is then freed unconditionally before the return (35c sub-class A,
        // plans/captured-group-elem-uaf.md). Skip trailing frees to reach the real value.
        Value::Block(bl) => {
            if let Some(last) = last_non_free_result(&bl.operators, data) {
                collect_return_sources(last, data, out);
            }
        }
        Value::Return(inner) | Value::Drop(inner) => collect_return_sources(inner, data, out),
        Value::Insert(ops) => {
            if let Some(last) = last_non_free_result(ops, data) {
                collect_return_sources(last, data, out);
            }
        }
        Value::If(_, t, f) => {
            collect_return_sources(t, data, out);
            collect_return_sources(f, data, out);
        }
        Value::Span(b) => collect_return_sources(&b.1, data, out),
        _ => {}
    }
}

/// The last operator of a sequence that carries the block's VALUE — i.e. skipping
/// trailing scope-exit frees (`OpFreeRef` / `OpFreeText` / `OpFreeRefIfDistinct`) and
/// `Line` position markers. A value-returning block can end in a free of a block-local
/// (a captured `_mv_<f>` text, a `..rest` `__vdb`) appended after the result, so the
/// naive `.last()` would mistake that free for the value.
pub(super) fn last_non_free_result<'a>(ops: &'a [Value], data: &Data) -> Option<&'a Value> {
    ops.iter()
        .rev()
        .find(|op| scope_free_op_var(op, data).is_none() && !matches!(op.unspan(), Value::Line(_)))
}

/// loft#1515 shape 2 — move a join-return's source hooks INTO its arms, so the arm is the path.
///
/// `materialize_return_into` publishes one copy of a join (`return if c { src.h } else { L }`),
/// and the sources feeding those arms are released once, after it, for every path alike. That
/// single sweep cannot be right: on the arm that hands `src.h` out, `src` must release its
/// OTHER members and leave that one to the copy; on the arm that does not, it must release the
/// whole record. `(O-Complete)` makes the ownership fact per path, and here the ARM *is* the
/// path — so no runtime witness is needed, which is loft#1476's shape one level over.
///
/// Only the HOOK moves. The store's `OpFreeRef` stays in the common sweep, because a store is
/// freed once whichever arm ran, and splitting that would free it per path.
///
/// Returns the locals whose hook this rewrote, for the sweep to skip. An empty answer leaves
/// everything exactly as it was.
pub(super) fn move_join_hooks_into_arms(
    expr: &mut Value,
    function: &Function,
    data: &Data,
) -> HashSet<u16> {
    let mut done = HashSet::default();
    let copy_nr = data.def_nr("OpCopyRecord");
    let get_field_nr = data.def_nr("OpGetField");
    let Some((join, armed)) = materialised_join_mut(expr, copy_nr) else {
        return done;
    };
    let mut arms = Vec::new();
    let is_join = join_arm_hand_offs(join, function, get_field_nr, &mut arms);
    if !is_join {
        return done; // not a join: `return_copy_out` owns the single-arm case
    }
    // The sources any arm speaks for. A local that no arm mentions keeps the common sweep.
    let mut sources: Vec<u16> = Vec::new();
    for a in &arms {
        let v = match a {
            ArmHandOff::Member(v, _) | ArmHandOff::Whole(v) => *v,
            ArmHandOff::Nothing => continue,
        };
        if !sources.contains(&v) {
            sources.push(v);
        }
    }
    sources.retain(|&v| drop_hook(function, v, data).is_some());
    if sources.is_empty() {
        return done;
    }
    if !armed {
        inject_per_arm(join, function, data, get_field_nr, &sources);
    }
    done.extend(sources);
    done
}

/// Walk the join's arms in the same order [`join_arm_hand_offs`] read them, giving each the
/// hooks its own path owes.
fn inject_per_arm(
    value: &mut Value,
    function: &Function,
    data: &Data,
    get_field_nr: u32,
    sources: &[u16],
) {
    if let Value::If(_, t, f) = value.unspan_mut() {
        inject_per_arm(t, function, data, get_field_nr, sources);
        inject_per_arm(f, function, data, get_field_nr, sources);
        return;
    }
    let mine = arm_hand_off(value, function, get_field_nr);
    let mut ops = Vec::new();
    for &v in sources {
        // This arm hands the whole record out: it MOVED, so this arm owes nothing for it
        // (`(H-Move)`).  An arm that hands out one MEMBER made a copy, which the rules refuse or
        // which leases a structure of its own (@PLN163 P6), so the record still owes its whole
        // release there — as does an arm that hands out nothing of `v`.
        if matches!(mine, ArmHandOff::Whole(w) if w == v) {
            continue;
        }
        if let Some(hook) = drop_hook(function, v, data) {
            ops.push(hook);
        }
    }
    inject_before_arm_value(value, &ops);
}

/// The name a `materialized_view_return` block takes once its join's hooks have been moved
/// into the arms.
///
/// The scan runs in more than one phase over a body the previous phase installed, so the
/// rewrite has to be IDEMPOTENT — without a marker the second phase injects a second copy of
/// every hook and the delivering arm releases twice. The name is the marker because it
/// travels with the node, unlike a set keyed on a variable number that a re-derived `Scopes`
/// starts empty.
const ARMED_VIEW_RETURN: &str = "materialized_view_return_armed";

/// The join feeding a `materialized_view_return`'s copy, if the return has that shape.
///
/// Answers for the ARMED name too, so a later phase can still learn which locals the arms
/// speak for without rewriting anything again.
fn materialised_join_mut(expr: &mut Value, copy_nr: u32) -> Option<(&mut Value, bool)> {
    fn tail(v: &mut Value) -> &mut Value {
        match v {
            Value::Span(_) | Value::Return(_) => {}
            Value::Insert(ops) if !ops.is_empty() => {}
            _ => return v,
        }
        match v {
            Value::Span(b) => tail(&mut b.1),
            Value::Return(inner) => tail(inner),
            Value::Insert(ops) => {
                let i = ops.len() - 1;
                tail(&mut ops[i])
            }
            other => other,
        }
    }
    let Value::Block(bl) = tail(expr) else {
        return None;
    };
    if bl.name != "materialized_view_return" && bl.name != ARMED_VIEW_RETURN {
        return None;
    }
    let armed = bl.name == ARMED_VIEW_RETURN;
    bl.name = ARMED_VIEW_RETURN;
    bl.operators
        .iter_mut()
        .find_map(|op| match op {
            Value::Call(d, args) if *d == copy_nr && !args.is_empty() => Some(&mut args[0]),
            _ => None,
        })
        .map(|j| (j, armed))
}

/// Put `ops` into one arm, as STATEMENTS before the value it yields.
///
/// The same placement rule [`free_record_in_omitting_arms`] states: a block takes a release as
/// a preceding statement, never as a wrapper around its value, because a wrapper puts an
/// `Insert` in value position and `--native` then emits the release itself as the block's
/// result.
fn inject_before_arm_value(arm: &mut Value, ops: &[Value]) {
    if ops.is_empty() {
        return;
    }
    match arm {
        Value::Span(b) => inject_before_arm_value(&mut b.1, ops),
        Value::Block(bl) if !bl.operators.is_empty() => {
            let idx = bl.operators.len() - 1;
            for (n, op) in ops.iter().enumerate() {
                bl.operators.insert(idx + n, op.clone());
            }
        }
        Value::Insert(list) if !list.is_empty() => {
            let idx = list.len() - 1;
            for (n, op) in ops.iter().enumerate() {
                list.insert(idx + n, op.clone());
            }
        }
        leaf => {
            let held = std::mem::replace(leaf, Value::Null);
            let mut list = ops.to_vec();
            list.push(held);
            *leaf = Value::Insert(list);
        }
    }
}

/// What ONE arm of a join-return hands out — loft#1515 shape 2.
///
/// The value an arm yields is copied into the return buffer, and `@FR-H-Drop` moves the
/// release with that copy. Which release moves is a fact about the ARM, so it is read per arm
/// and never unioned: an arm handing out `src.h` leaves `src`'s OTHER members to `src`, and a
/// sibling arm handing out nothing leaves `src` the whole record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArmHandOff {
    /// The arm yields a MEMBER of this local, at this `(offset, depth)` path.
    Member(u16, (u16, u16)),
    /// The arm yields this whole frame-owned record, so its release moves entire.
    Whole(u16),
    /// The arm yields something no local owns — a literal built elsewhere, a call result.
    Nothing,
}

/// Read [`ArmHandOff`] off one arm's value.
///
/// A block or insert is judged by its TAIL, which is the value the arm yields; the ops before
/// it built it. `Nothing` is the safe answer and the fallback, because it leaves every source
/// releasing exactly what it releases today.
fn arm_hand_off(arm: &Value, function: &Function, get_field_nr: u32) -> ArmHandOff {
    match arm.unspan() {
        Value::Block(bl) => bl.operators.last().map_or(ArmHandOff::Nothing, |t| {
            arm_hand_off(t, function, get_field_nr)
        }),
        Value::Insert(ops) => ops.last().map_or(ArmHandOff::Nothing, |t| {
            arm_hand_off(t, function, get_field_nr)
        }),
        Value::Var(v) if !function.is_argument(*v) => ArmHandOff::Whole(*v),
        Value::Call(gf, _) if *gf == get_field_nr => match projection_root(arm, get_field_nr) {
            Some((src, path)) if !function.is_argument(src) => ArmHandOff::Member(src, path),
            _ => ArmHandOff::Nothing,
        },
        _ => ArmHandOff::Nothing,
    }
}

/// Every arm of a join, in order, with what each hands out.
///
/// Returns `None` when the value is not a join — the single-arm case is
/// [`return_copy_out`]'s and needs no per-arm rewrite.
fn join_arm_hand_offs(
    value: &Value,
    function: &Function,
    get_field_nr: u32,
    out: &mut Vec<ArmHandOff>,
) -> bool {
    match value.unspan() {
        Value::If(_, t, f) => {
            join_arm_hand_offs(t, function, get_field_nr, out);
            join_arm_hand_offs(f, function, get_field_nr, out);
            // TRUE because THIS node is the join, whatever its arms turned out to be — a
            // nested `if` answers for its own level and a leaf answers `false`, so returning
            // the children's answer made every two-leaf join read as "not a join".
            true
        }
        other => {
            out.push(arm_hand_off(other, function, get_field_nr));
            false
        }
    }
}

/// loft#1623, `(H-Move)` — the frame-owned TEMPS a return hands out, when the value it copies
/// onto the return buffer is a joined binding that BORROWS them.
///
/// `(H-Move)`'s third clause ends a function's own variables with it, and a binding that
/// declined the per-arm write-out is not one of them — it releases nothing and the temps behind
/// it do.  So the return moves what THEY hold, and their hook belongs to the caller from there
/// on.  [`return_copies_whole_local`] answers the owned-local spelling of the same question and
/// declines a view, correctly: the view owes no release, and naming it would suppress nothing.
///
/// PER EXIT, which is the whole of why this is read here rather than recorded as a hand-off: a
/// second exit that returns something ELSE leaves the temps holding a value nobody took, and
/// there their hook is the one release it gets (guard cell `c7`).
///
/// Only the temps [`Scopes::join_holders`] recorded.  A container the frame keeps — `s` on the
/// path where `x = s.h ?? b` chose `s.h` — is not one: the caller got a COPY of a member, and
/// `s` still owes its own release (guard cell `c4`).
pub(super) fn return_copies_view_holders(
    expr: &Value,
    function: &Function,
    data: &Data,
    holders: &HashMap<u16, Vec<u16>>,
) -> Vec<u16> {
    return_copied_local(expr, function, data)
        .and_then(|src| holders.get(&src).cloned())
        .unwrap_or_default()
}

/// The frame's own local a return COPIES onto the return buffer — the source of the
/// `OpCopyRecord` inside a `materialized_view_return` — or `None` for any other return.
fn return_copied_local(expr: &Value, function: &Function, data: &Data) -> Option<u16> {
    let Value::Block(bl) = return_tail(expr) else {
        return None;
    };
    if bl.name != "materialized_view_return" && bl.name != ARMED_VIEW_RETURN {
        return None;
    }
    let copy_nr = data.def_nr("OpCopyRecord");
    bl.operators.iter().find_map(|op| {
        let Value::Call(d, args) = op.unspan() else {
            return None;
        };
        if *d != copy_nr {
            return None;
        }
        let (Value::Var(src), Some(Value::Var(dest))) =
            (args.first()?.unspan(), args.get(1).map(Value::unspan))
        else {
            return None;
        };
        (src != dest && !function.is_argument(*src)).then_some(*src)
    })
}

/// loft#1628, `@FR-H-Move` — the holders whose record a return may be handing out, when it copies
/// a WITNESSED local onto the return buffer: the local's witness (a record it minted, after a
/// rebind) and the plain locals it was bound to as they are (a value branch's arm).
///
/// Which of them holds the returned record is a per-RUN fact — `x: H = s.h ?? b; if c
/// { x = mk(9) }; return x` hands out `mk(9)`'s record through the witness on one run and `b`'s
/// on the other — so each holder's hook is guarded on RECORD identity with the returned local
/// (`OpNeRef`), not suppressed.  Record identity and not store identity: a container whose
/// MEMBER the local views shares its store and still owes its own release (#1623's `c4`).
pub(super) fn return_moved_holders(
    expr: &Value,
    function: &Function,
    data: &Data,
    witness: &HashMap<u16, u16>,
    aliases: &HashMap<u16, Vec<u16>>,
) -> Option<(u16, Vec<u16>)> {
    let x = return_copied_local(expr, function, data)?;
    let &w = witness.get(&x)?;
    let mut holders = vec![w];
    holders.extend(aliases.get(&x).into_iter().flatten().copied());
    Some((x, holders))
}

/// D-heap-7 — the whole LOCAL a return copies onto the return buffer, when the copy owns that
/// local's release.
///
/// The twin of [`return_copy_out`] for the whole record rather than one member: `return b`
/// becomes `{ OpDatabase(buf); OpCopyRecord(b, buf); buf }` when another return already made
/// the buffer a local of its own.  Only a local that owns its record qualifies — a parameter's
/// record is the caller's, a VIEW (a local with deps, or one `views` names) releases nothing of
/// its own — and a copy onto itself is no copy.  `None` keeps the source's full hook, which is
/// the double release this replaces and never a lost one.
pub(super) fn return_copies_whole_local(
    expr: &Value,
    function: &Function,
    data: &Data,
    views: &HashMap<u16, (u16, (u16, u16))>,
) -> Option<u16> {
    let Value::Block(bl) = return_tail(expr) else {
        return None;
    };
    if bl.name != "materialized_view_return" && bl.name != ARMED_VIEW_RETURN {
        return None;
    }
    let copy_nr = data.def_nr("OpCopyRecord");
    bl.operators.iter().find_map(|op| {
        let Value::Call(d, args) = op.unspan() else {
            return None;
        };
        if *d != copy_nr {
            return None;
        }
        let (Value::Var(src), Some(Value::Var(dest))) =
            (args.first()?.unspan(), args.get(1).map(Value::unspan))
        else {
            return None;
        };
        // @FR-O-Proxy asks free — through `proxy_says_owned`, which carries @FR-O-Override's
        // veto: the answer decides whose release this is, and a borrow owes none.
        (src != dest
            && !function.is_argument(*src)
            && !views.contains_key(src)
            && function.proxy_says_owned(*src)
            && drop_hook(function, *src, data).is_some())
        .then_some(*src)
    })
}

/// The value a return yields: through its span, the `Return` itself and the statements an
/// `Insert` runs before its last element.
fn return_tail(v: &Value) -> &Value {
    match v {
        Value::Span(b) => return_tail(&b.1),
        Value::Return(inner) => return_tail(inner),
        Value::Insert(ops) if !ops.is_empty() => return_tail(&ops[ops.len() - 1]),
        other => other,
    }
}

/// The `(variable, (byte offset, depth))` an `OpGetField` chain reads — the PATH from a
/// variable's record to the member it names.
///
/// Struct members are laid out INSIDE the record that owns them (`formal/layout.md`), so a
/// chain of field reads is one address and its offsets add up; the depth is how many links
/// the chain has below the first.  Both are needed because a member at offset 0 shares its
/// owner's address, so an offset alone cannot say WHICH of them the chain named.
///
/// The TUPLE spelling (`TupleGet`) is deliberately absent: a tuple member's backing is
/// carried by `tuple_member_backing` and released by `tuple_owned_elem_frees`, which is
/// D-heap-1's mechanism with its own pairing — recognising it here too would give one member
/// two claimants.  This reader answers only for the inline struct members `OpGetField` names.
///
/// `None` for a base that is not a plain variable — an element read, a call temporary or a
/// keyed lookup names a place this frame's scope-end release does not reach.
pub(super) fn projection_root(expr: &Value, get_field_nr: u32) -> Option<(u16, (u16, u16))> {
    let Value::Call(gf, gargs) = expr.unspan() else {
        return None;
    };
    if *gf != get_field_nr {
        return None;
    }
    let Value::Int(off) = gargs.get(1)?.unspan() else {
        return None;
    };
    let off = u16::try_from(*off).ok()?;
    match gargs.first()?.unspan() {
        Value::Var(src) => Some((*src, (off, 0))),
        base => {
            let (src, (outer, depth)) = projection_root(base, get_field_nr)?;
            Some((src, (outer.checked_add(off)?, depth.checked_add(1)?)))
        }
    }
}

pub(super) fn return_has_non_source_arm(expr: &Value, sources: &[u16]) -> bool {
    fn walk(e: &Value, sources: &[u16], in_join: bool) -> bool {
        match e.unspan() {
            Value::If(_, t, f) => walk(t, sources, true) || walk(f, sources, true),
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|o| walk(o, sources, in_join)),
            Value::Insert(ops) => ops.last().is_some_and(|o| walk(o, sources, in_join)),
            Value::Return(inner) | Value::Drop(inner) => walk(inner, sources, in_join),
            Value::Var(v) => in_join && !sources.contains(v),
            _ => in_join,
        }
    }
    walk(expr, sources, false)
}

/// @PLN85 A.1 part i — does this return expression have a reachable arm whose
/// terminal is the typed-null sentinel (`OpNullRefSentinel` / `Value::Null`)?
///
/// A nullable `if b { Struct{} } else { null }` allocates the present arm's
/// work-ref placeholder (`__ref_N`) at function entry, but the `null` arm
/// returns the sentinel and leaves that placeholder ORPHANED.  The SET-driven
/// free-suppression must therefore NOT suppress that work-ref: whether it is
/// transferred (present path) or orphaned-and-must-free (null path) is a RUNTIME
/// decision, not a static one — so the SET hands it back to the standard
/// work-ref free path (which frees the orphan correctly).  Resolving the runtime
/// split (NRVO-delivering each arm into `__retbuf`, or a conditional free) is the
/// callee-side control.rs / native work, out of scope-analysis's reach.
///
/// `null_sentinel_nr` is `OpNullRefSentinel`'s def number (resolved by the
/// caller, which holds `data`).
///
/// A `Return`/`Drop` wrapper IS descended, because this is asked about a return
/// EXPRESSION and the wrapper is the thing being examined.  The `parser::control`
/// siblings (`branch_yields_null`, `arm_yields_direct_null`, `arm_is_null`) ask the same
/// null question about what an ARM hands to a JOIN — where a `return` hands it nothing —
/// and stop at the wrapper.
///
/// One home, because the same fact decides two things at opposite ends of one return:
/// whether scope analysis may suppress the work-ref's free (here), and whether the
/// `Bind` leg may copy the tail into the return buffer WHOLE or has to deliver the arms
/// one at a time (`ref_return`) — the whole-tail copy answers the buffer on every path,
/// so with a null arm present it swallows the sentinel.
pub(crate) fn return_has_null_arm(expr: &Value, null_sentinel_nr: u32) -> bool {
    match expr.unspan() {
        Value::Null => true,
        Value::Call(d, _) => *d == null_sentinel_nr,
        Value::If(_, t, f) => {
            return_has_null_arm(t, null_sentinel_nr) || return_has_null_arm(f, null_sentinel_nr)
        }
        Value::Block(bl) => bl
            .operators
            .last()
            .is_some_and(|o| return_has_null_arm(o, null_sentinel_nr)),
        Value::Insert(ops) => ops
            .last()
            .is_some_and(|o| return_has_null_arm(o, null_sentinel_nr)),
        Value::Return(inner) | Value::Drop(inner) => return_has_null_arm(inner, null_sentinel_nr),
        _ => false,
    }
}
