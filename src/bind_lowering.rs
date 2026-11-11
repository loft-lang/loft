// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! How an assignment is LOWERED — the one decision of what `Set(v, value)` does to stores
//! (doc/claude/STORE_RECORDS.md § What the IR does not say yet).
//!
//! An IR `Set` does not say whether it binds `v` to the value, deep-copies the value into a
//! fresh store of `v`'s own, adopts a store a callee minted, or copies-or-adopts per run.  The
//! code generator decides, and the store records need the same answer.  This module is where
//! the decision lives, so both read ONE answer: the code generator emits the arm this names,
//! and the records give the arm its store meaning.
//!
//! The arms carry the names `LOFT_TRACE_BIND` prints.

use crate::data::{Data, Type, Value};
use crate::variables::Function;

/// The lowering of a FIRST assignment `v = value` (the variable has no slot yet).  The order
/// of the tests in [`first_arm`] is the order of the old condition chain, and it decides:
/// a later arm is reached only when every earlier one declined.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FirstArm {
    /// A text local: its own path, not a store.
    Text,
    /// `F1` — a reference local's null init: the sentinel, no store.
    RefNull,
    /// An iterator handle's null init.
    IterNull,
    /// `F2` — `v = OpCopyRecord(..)`: a fresh store of `v`'s own, filled by the copy.
    RecordCopyCall { d_nr: u32 },
    /// `F3` — `v = src`, both one heap record type: `v` gets its own record (a copy, or a
    /// move where `src` is not read again).
    RecordVarCopy { src: u16, d_nr: u32 },
    /// `F4` — an element read bound to a local that owns: materialised into a fresh store.
    ElementCopy { d_nr: u32 },
    /// `F5` — a tuple element bound to a local that owns: a deep copy into a fresh store.
    TupleElementCopy { d_nr: u32 },
    /// `F6` — a closure call whose answer may borrow `base`: copied when the answer is
    /// `base`'s store, adopted when the callee minted it (`OpBindOrCopy`).
    CallRefJoin { d_nr: u32, base: u16 },
    /// `F7-copy` — a nullable bind from a call that may answer any of several arguments:
    /// copied into a fresh store on every run.
    NullableCallCopy { d_nr: u32 },
    /// `F7-join` — a nullable bind from a call that may answer `base`: copy or adopt per run.
    NullableCallJoin { d_nr: u32, base: u16 },
    /// `F8` — a closure call whose target is unresolved: copy a store it hands back that is
    /// not its own, adopt one it minted (`OpBindFnRefResult`).
    OpaqueCallRef { rec: u32 },
    /// `F9a` — a call whose answer is a runtime join on `base`: copy or adopt per run.
    CallJoin { d_nr: u32, base: u16 },
    /// `F9b` — a call answering a fresh store: `v` adopts it.
    CallAdopt,
    /// `F9c` — a call answering a store `v` may not take: deep-copied into a fresh store.
    CallCopy { d_nr: u32 },
    /// `F10` — a nullable vector's null init.
    NullableVectorNull,
    /// `F11` — a vector's null init.
    VectorNull,
    /// `F12` — a keyed collection's null init.
    KeyedNull,
    /// A tuple's null init.
    TupleNull,
    /// A function reference: not a store.
    FnRef,
    /// `v = src`, both `&` links: the cell is copied, no store.
    RefVar,
    /// `F13` — everything else: the value is pushed and `v` binds to it as it is.
    Plain,
}

impl FirstArm {
    /// The name `LOFT_TRACE_BIND` prints for this arm.
    #[must_use]
    pub fn trace_name(self) -> &'static str {
        match self {
            FirstArm::Text => "first-text",
            FirstArm::RefNull => "F1",
            FirstArm::IterNull => "first-iter-null",
            FirstArm::RecordCopyCall { .. } => "F2",
            FirstArm::RecordVarCopy { .. } => "F3",
            FirstArm::ElementCopy { .. } => "F4",
            FirstArm::TupleElementCopy { .. } => "F5",
            FirstArm::CallRefJoin { .. } => "F6",
            FirstArm::NullableCallCopy { .. } => "F7-copy",
            FirstArm::NullableCallJoin { .. } => "F7-join",
            FirstArm::OpaqueCallRef { .. } => "F8",
            FirstArm::CallJoin { .. } => "F9a",
            FirstArm::CallAdopt => "F9b",
            FirstArm::CallCopy { .. } => "F9c",
            FirstArm::NullableVectorNull => "F10",
            FirstArm::VectorNull => "F11",
            FirstArm::KeyedNull => "F12",
            FirstArm::TupleNull => "first-tuple-null",
            FirstArm::FnRef => "first-fn",
            FirstArm::RefVar => "first-refvar",
            FirstArm::Plain => "F13",
        }
    }
}

/// The lowering of the first assignment `v = value` in function `d_nr`; the reason for each
/// test is beside it.  `state::codegen::gen_set_first_at_tos` emits the answer.
#[must_use]
pub fn first_arm(data: &Data, d_nr: u32, function: &Function, v: u16, value: &Value) -> FirstArm {
    let tp = function.tp(v);
    // A `text?` local routes to the text path too: `.base()` peels the marker (@PLN25).
    if matches!(tp.base(), Type::Text(_)) {
        return FirstArm::Text;
    }
    // `.base()`: a `τ?` local's null init is the same sentinel write as `τ`'s.  Asked bare, an
    // `Optional(Reference)` temp fell to the plain bind, which lands the shared null store in
    // the slot, and its scope-exit free then trips the stack-store guard (#306).
    if matches!(tp.base(), Type::Reference(_, _) | Type::Enum(_, true, _)) && *value == Value::Null
    {
        return FirstArm::RefNull;
    }
    // `z: iterator<τ>? = null` is a null HANDLE, the sentinel a nullable collection writes.
    if matches!(tp.base(), Type::Iterator(_, _)) && *value == Value::Null {
        return FirstArm::IterNull;
    }
    if let Type::Reference(rec, _) = tp
        && let Value::Call(op_nr, _) = value.unspan()
        && data.def(*op_nr).name() == "OpCopyRecord"
    {
        return FirstArm::RecordCopyCall { d_nr: *rec };
    }
    // `@FR-B-Copy` — a whole-value bind of a heap record is INDEPENDENT: `d = c` gives `d` its
    // own record.  Asked through `heap_def_nr`, so a struct-enum copies as a struct does, and
    // through `base()`, so `S?` (the same storage, @FR-L-Null) copies too (loft#1319).
    // `record_copy_source` reads a `&S` source as its `S` (@FR-C-Ref).
    if let Some(rec) = tp.base().heap_def_nr()
        && let Value::Var(src) = value
        && let Some(src_d_nr) = function.record_copy_source(data, v, *src)
        && data.copies_as(rec, src_d_nr)
    {
        return FirstArm::RecordVarCopy {
            src: *src,
            d_nr: rec,
        };
    }
    // `@FR-O-Proxy` — an element read bound to a local whose deps are EMPTY (it owns: the deps
    // were stripped because it is reassigned later) is MATERIALISED into a store of its own.
    // Bound as the interior pointer, the reassignment's free would release the CONTAINER
    // (loft#778); a binding that kept its deps still aliases.  Not for a WITNESSED local
    // (@FR-O-Witness, loft#1336): its deps are empty only because a later copy stripped them.
    // Through `heap_def_nr` and `base()`, so a struct-enum or nullable element view
    // materialises too (loft#1693, loft#1456).
    if let Some(rec) = tp.base().heap_def_nr()
        && tp.depend().is_empty()
        && function.owner_witness(v).is_none()
        && crate::use_analysis::projection_container_var(data, value).is_some()
    {
        return FirstArm::ElementCopy { d_nr: rec };
    }
    // `@FR-O-Proxy` — a tuple element bound to a local that OWNS (empty deps) is deep-copied;
    // one whose deps say borrow aliases, or the copy has an owner nobody frees (loft#1051).
    if let Type::Reference(rec, _) = tp
        && let Value::TupleGet(_, _) = value
        && tp.depend().is_empty()
        && function.owner_witness(v).is_none()
    {
        return FirstArm::TupleElementCopy { d_nr: *rec };
    }
    // A CLOSURE call whose answer may borrow: copy when the answer is the witness's store,
    // adopt when the closure minted it (loft#1248).
    if let Some((rec, base)) = crate::use_analysis::callref_join_first_bind(data, d_nr, tp, value) {
        return FirstArm::CallRefJoin { d_nr: rec, base };
    }
    // A NULLABLE heap local first bound from a call that may answer an argument: the `-> S`
    // twin's per-run copy-or-adopt, or a copy on every run when the borrow is of one of
    // several arguments (loft#1106, loft#1550).
    if let Some((rec, base)) = crate::use_analysis::nullable_join_first_bind(data, d_nr, tp, value)
    {
        return if base == u16::MAX {
            FirstArm::NullableCallCopy { d_nr: rec }
        } else {
            FirstArm::NullableCallJoin { d_nr: rec, base }
        };
    }
    // A closure call whose target is unresolved: copy a store it hands back that is a
    // capture, adopt one it minted (@FR-O-Unknown decides, loft#1659).
    if let Some(rec) = crate::use_analysis::opaque_callref_bind(data, d_nr, tp, value) {
        return FirstArm::OpaqueCallRef { rec };
    }
    // A call to a loft-defined function (`Call` and a `CallRef` that reaches a definition,
    // loft#1245; methods and monomorphs too, loft#810), into a heap record local: a runtime
    // JOIN answer copies-or-adopts per run (@PLN85); a FRESH answer is adopted (also when the
    // view is elided, @PLN157 § V-g, or the callee returns the local it promoted onto its
    // buffer, @FR-O-Move); any other answer is tied to an argument or a reused buffer and is
    // deep-copied (#429 for the struct-enum).
    if let Type::Reference(rec, _) | Type::Enum(rec, true, _) =
        crate::use_analysis::first_bind_shape(tp).clone()
        && let Some(fn_nr) = crate::use_analysis::callee_of(data, d_nr, value)
        && data.def(fn_nr).is_loft_defined()
    {
        if crate::keys::join_own_enabled()
            && let Type::Reference(join_d_nr, _) = crate::use_analysis::first_bind_shape(tp).clone()
            && !function.is_view_elided(v)
            && let crate::use_analysis::Own::Join { base } =
                crate::use_analysis::ownership_of(data, d_nr, value)
            && base != u16::MAX
        {
            return FirstArm::CallJoin {
                d_nr: join_d_nr,
                base,
            };
        }
        if data.def(fn_nr).return_adopts_fresh_store()
            || function.is_view_elided(v)
            || crate::use_analysis::adopts_minted_at_bind(data, function, v, value)
        {
            return FirstArm::CallAdopt;
        }
        return FirstArm::CallCopy { d_nr: rec };
    }
    first_arm_plain(function, tp, value)
}

/// The arms of a first assignment that is not a heap record bind: the null inits, a function
/// reference, an `&` link, and the plain bind.
fn first_arm_plain(function: &Function, tp: &Type, value: &Value) -> FirstArm {
    let value = value.unspan();
    if *value == Value::Null
        && matches!(tp, Type::Optional(_))
        && matches!(tp.base(), Type::Vector(_, _))
    {
        return FirstArm::NullableVectorNull;
    }
    if matches!(tp, Type::Vector(_, _)) && *value == Value::Null {
        return FirstArm::VectorNull;
    }
    if crate::parser::vectors::owns_keyed_store(tp) && *value == Value::Null {
        return FirstArm::KeyedNull;
    }
    if matches!(tp.base(), Type::Tuple(_)) && *value == Value::Null {
        return FirstArm::TupleNull;
    }
    if matches!(tp, Type::Function(..)) {
        return FirstArm::FnRef;
    }
    if matches!(tp, Type::RefVar(_))
        && let Value::Var(src) = value
        && matches!(function.tp(*src), Type::RefVar(_))
    {
        return FirstArm::RefVar;
    }
    FirstArm::Plain
}

/// How a reassignment releases the store it displaces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DisplacedFree {
    /// No free at this assignment.
    None,
    /// The old reference is stashed before the value is built and freed after it, unless the
    /// new value names the same store (`free_stashed`): the value reads `v`, builds a record
    /// into a container `v` may view, or `v` is nullable.
    Stash,
    /// Freed before the value is built.  `witness` (a projection's container, or the hidden
    /// buffer the right-hand call is handed) keeps it when it is the witness's store.
    Pre { witness: Option<u16> },
}

/// The lowering of a REASSIGNMENT `v = value` (the variable holds a slot already), after the
/// displaced store's free.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReArm {
    /// `B0` — a keyed local assigned null: cleared in place.
    KeyedNull,
    /// `R1` — a runtime JOIN answer on `base`: `OpBindOrCopy` adopts the owned arm and copies
    /// the borrowed one.
    Join { d_nr: u32, base: u16 },
    /// `R2a` — a closure call with an unresolved target: copy-or-adopt
    /// (`OpBindFnRefResult`) into a fresh store.
    OpaqueCallRef { rec: u32 },
    /// `R2b` — a call whose answer `v` may not take: deep-copied into a store of `v`'s own.
    CallCopy { d_nr: u32, fn_nr: u32 },
    /// `R2c` — `v = src`, one heap record type: copied (`OpBindOrCopy` with `src` as witness).
    VarCopy { src: u16, d_nr: u32 },
    /// `R2d` — an element read into a local that owns: materialised into a fresh store.
    ElementCopy { d_nr: u32 },
    /// `R3` — `set_var`: the value is pushed and `v` binds to it as it is.
    Plain,
}

impl ReArm {
    /// The name `LOFT_TRACE_BIND` prints for this arm.
    #[must_use]
    pub fn trace_name(self) -> &'static str {
        match self {
            ReArm::KeyedNull => "B0",
            ReArm::Join { .. } => "R1",
            ReArm::OpaqueCallRef { .. } => "R2a",
            ReArm::CallCopy { .. } => "R2b",
            ReArm::VarCopy { .. } => "R2c",
            ReArm::ElementCopy { .. } => "R2d",
            ReArm::Plain => "R3",
        }
    }
}

/// Everything a reassignment's lowering is decided on, and the decision.  The facts are
/// public because the emission reads them too: where the copy allocates, which arguments are
/// bracketed, how the stashed free is witnessed.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts, each read on its own"
)]
#[derive(Clone, Copy, Debug)]
pub struct ReBind {
    pub arm: ReArm,
    pub free: DisplacedFree,
    /// The right-hand call writes into `v` itself: `v` is passed at a HIDDEN parameter
    /// position (`nrvo_collapse_tail_set`'s substitution).
    pub s1_substituted: bool,
    /// `v` is this function's own hidden buffer parameter, handed on to that call.
    pub is_hidden_buf_arg: bool,
    /// The value reads `v` (`@FR-O-Detach`).
    pub rhs_reads_v: bool,
    /// The oracle's borrow base of the value, when it borrows or joins.
    pub rhs_base: Option<u16>,
    /// The value may name `v`'s own store.
    pub rhs_may_alias_v: bool,
    /// The entry witness of a promoted return buffer (`@FR-O-Buffer`).
    pub entry_w: Option<u16>,
    /// `v` owns the store it displaces, so this assignment frees it (`@FR-O-Proxy`).
    pub owned_ref: bool,
    /// The value is an `OpNewRecord`: an interior record of a container.
    pub rhs_is_new_record: bool,
    /// `v` is nullable.
    pub nullable_local: bool,
    /// `v` has an owner witness that releases its stores (`@FR-O-Witness`).
    pub witnessed: bool,
    /// `v` adopted a construction buffer's store, which that buffer releases (loft#1522).
    pub buffer_kept: bool,
    /// The hidden buffer the right-hand call is handed, as a witness of the displaced free.
    pub buffer_witness: Option<u16>,
}

impl ReBind {
    /// Does a copying reassignment (`ReArm::CallCopy`) land in a FRESH store, rather than
    /// re-initialise `v`'s own store in place?  The ONE answer the interpreter's code generator
    /// emits (`Codegen::generate_set_inner`, arm R2b) and the store records read: a witnessed
    /// local whose value reads it, and wherever the displaced store waits for its stashed free
    /// and the value may alias `v` — or does not read `v` at all (`LOFT_NO_STASH_FRESH`).
    #[must_use]
    pub fn copies_into_fresh(&self) -> bool {
        (self.witnessed && self.rhs_reads_v)
            || (self.free == DisplacedFree::Stash
                && (self.rhs_may_alias_v || (!self.rhs_reads_v && crate::keys::stash_fresh())))
    }

    /// Is `v`'s OLD store protected across the copy (`n_protect_store_frees` through a stashed
    /// reference), so the copy's source-free cannot release it?  Where `v` is one of the call's
    /// protectable reference arguments (`v_is_ref_arg`) and the value reads it: a witnessed
    /// local, and a copy into a fresh store — there the stashed free is the old store's one
    /// release (`LOFT_NO_FRESH_PROTECTS_OLD` leaves it out of the bracket).
    #[must_use]
    pub fn protects_old(&self, v_is_ref_arg: bool) -> bool {
        self.rhs_reads_v
            && v_is_ref_arg
            && (self.witnessed
                || (self.copies_into_fresh() && !crate::keys::fresh_protects_old_off()))
    }
}

/// Is `v` the function's return buffer with a local's name — a hidden parameter other than
/// `__retbuf`?
#[must_use]
pub(crate) fn is_promoted_return_buffer(
    data: &Data,
    d_nr: u32,
    function: &Function,
    v: u16,
) -> bool {
    let name = function.name(v);
    name != "__retbuf"
        && data
            .def(d_nr)
            .attr_names
            .get(name)
            .is_some_and(|&a| data.def(d_nr).attributes()[a].hidden)
}

/// loft#2021 — a return temp (`__ret_N`) bound in a branch written out per arm, whose deps
/// name only hidden return buffers: each arm's bind copies into it, so it owns what an arm
/// displaces.
fn is_return_temp_owning_its_arms(data: &Data, d_nr: u32, function: &Function, v: u16) -> bool {
    let name = function.name(v);
    if !name.starts_with("__ret_") {
        return false;
    }
    let deps = function.tp(v).depend();
    !deps.is_empty()
        && deps.iter().all(|&d| {
            (d as usize) < function.count() as usize && {
                let n = function.name(d);
                let def = data.def(d_nr);
                n == "__retbuf"
                    || def
                        .attr_names
                        .get(n)
                        .is_some_and(|&a| def.attributes()[a].hidden)
            }
        })
}

/// Does `value` name `v` on a path that can complete — anywhere except inside an arm that
/// leaves by `return`, `break` or `continue`?  [`Value::reads_var`] with the exit arms skipped:
/// the frees a discharge's exit carries name every owned local, the destination of the bind
/// included, and none of them is a read the bound value depends on.
fn reads_var_unless_exiting(value: &Value, v: u16) -> bool {
    fn exits(node: &Value) -> bool {
        match node.unspan() {
            Value::Return(_) | Value::Break(_) | Value::Continue(_) => true,
            Value::Block(bl) => bl.operators.iter().any(exits),
            Value::Insert(ops) => ops.iter().any(exits),
            Value::If(_, t, e) => exits(t) && exits(e),
            _ => false,
        }
    }
    match value.unspan() {
        Value::If(c, t, e) => {
            reads_var_unless_exiting(c, v)
                || (!exits(t) && reads_var_unless_exiting(t, v))
                || (!exits(e) && reads_var_unless_exiting(e, v))
        }
        other => {
            other.names_var_here(v) || {
                let mut found = false;
                other.for_each_child(&mut |c| found = found || reads_var_unless_exiting(c, v));
                found
            }
        }
    }
}

/// `@FR-H-FreeTwice` — does `value` read a local that VIEWS `v`'s store (one whose deps name
/// `v`)?  Then the call may be handed the very store `v` displaces, so the displaced free waits
/// until after it, and runs only when the stores differ (`DisplacedFree::Stash`): `x = m(x)`
/// where `x` views a lift temp that owns each delivery.  `rhs_reads_v` is syntactic and sees
/// only `v` itself.
fn rhs_reads_a_view_of(function: &Function, value: &Value, v: u16) -> bool {
    (0..function.count())
        .any(|w| w != v && function.tp(w).depend().contains(&v) && value.reads_var(w))
}

/// The facts a reassignment `v = value` in function `d_nr` is lowered on, and the lowering;
/// the reason for each fact is beside it.  `state::codegen::generate_set_inner` emits the
/// answer.
#[must_use]
pub fn re_bind(data: &Data, d_nr: u32, function: &Function, v: u16, value: &Value) -> ReBind {
    // Free the old store before reassigning an owned Reference.
    // Only safe when the variable truly owns its store (dep empty).
    // The dep was cleared on first assignment only when codegen will
    // deep-copy (gen_set_first_ref_call_copy). O-B2 adopted stores
    // keep their dep, so this won't fire for them.
    // @P377-CORRUPTION: skip when this is an S1-substituted call
    // (`Set(v, Call(fn, [.., Var(v), ..]))` — the inner Call's
    // hidden-buffer arg has been substituted to be v itself by
    // `nrvo_collapse_tail_set`).  In that shape, the Call writes
    // its result INTO v's existing store, so freeing v's old
    // content here destroys what the Call is about to write into.
    // Detection (#360): an arg of the RHS Call is `Var(v)` AT A
    // HIDDEN parameter position — only the hidden buffer arg is
    // what `nrvo_collapse_tail_set` substitutes.  A VISIBLE
    // `Var(v)` argument (`s = grow(s)` passing s as plain data) is
    // NOT the S1 shape: classifying it as S1 skipped both the
    // pre-Set free and the deep-copy, so `s` adopted the call-site
    // buffer store and the next iteration's callee `OpDatabase`
    // wiped it while `x` still pointed there — every later
    // iteration read null.  Pairs with the evaluate-call-first
    // re-init order in the deep-copy emission below (the interp
    // half of @P290).  Bails for every other shape
    // (struct-literal RHS, non-S1 calls, non-tail patterns) —
    // those keep the load-bearing pre-Set FreeRef intact, so
    // `tests/scripts/130-gridmesh-crystal-equiv.loft` and the
    // broader moros_* suite stay green.
    let s1_substituted = if let Value::Call(fn_nr, args) = value.unspan() {
        let attrs = data.def(*fn_nr).attributes();
        args.iter().enumerate().any(|(i, a)| {
            matches!(a.unspan(), Value::Var(av) if *av == v)
                && attrs.get(i).is_some_and(|at| at.hidden)
        })
    } else {
        false
    };
    // @PLAN51 Cluster II/III — also skip the pre-Set OpFreeRef
    // when `v` is a hidden-buffer parameter AND the call IS S1-
    // substituted (the call's args contain `v` itself).  In that
    // case the callee's OpDatabase routes through
    // `state/io.rs::database` which does `clear(cv); claim(cv,
    // size)` — an in-place reuse of the caller's buffer store.
    // No leak, no corruption: store_nr is preserved.
    //
    // When the call is NOT S1-substituted (probes 04 + 28: the
    // call's hidden buf is a fresh `__ref_N` distinct from `v`),
    // the in-place-reuse assumption breaks — `v`'s current store
    // (e.g. from a preceding struct-literal init) becomes
    // orphan when the reassignment writes the deep-copied
    // result.  We must fall through to the reassignment path so
    // the pre-Set free runs on `v`.  `s1_substituted` is the
    // already-computed check we need.
    let is_hidden_buf_arg = s1_substituted
        && function.is_argument(v)
        && data
            .def(d_nr)
            .attributes()
            .iter()
            .any(|a| a.hidden && function.var(&a.name) == v);
    // Enforces @FR-O-Detach — this is the site that gets the order right, and the
    // one the other three are measured against: the free is DEFERRED past the
    // assignment when the RHS reads `v`, rather than the slot being prepared first.
    //
    // #330 — ONE recursive "does the RHS read v?" predicate decides
    // the free strategy (shared with scopes' #316 transition logic).
    // The old top-level-arg S1 scan missed struct-literal field
    // initialisers (`x = S { v: x.v + 1 }`) and the bare self-assign
    // (`x = x`): the pre-Set free released the store, the literal's
    // OpDatabase reused the slot, and the RHS read the recycled
    // record — silent null/corruption on both backends.
    //
    // When the RHS reads v, the old DbRef is STASHED on the eval
    // stack before anything runs and freed AFTER the assignment via
    // OpFreeRefIfDistinct — which also degrades to a no-op for the
    // S1 in-place shapes (the new value IS the old store).
    let rhs_reads_v = value.reads_var(v);
    // @FR-O-Detach — may the value being assigned name `v`'s OWN store, so that
    // re-initialising that store IN PLACE would wipe the record the copy reads from?
    //
    // Three facts were tried here and two of them are wrong in opposite directions;
    // the measurements are in `formal/ownership-history.md` D-own-41, and the short form is:
    //
    // - `value.reads_var(v)` is SYNTACTIC and answers NO for the defect, because a `??`
    //   HOISTS the read of `v` into a temporary in a PRIOR statement — the lowering
    //   `(O-Detach)` itself prescribes — so the assignment mentions `v` nowhere.  It also
    //   answers YES for `s = grow(s)`, which needs the in-place destination and leaks one
    //   store per loop pass without it.
    // - `Def::returns_borrowed_view` (the return's dep names a visible param) is true for
    //   a callee that MINTS its record and carries a dep through a `text` field, so it
    //   forces freshness on the same leaking shape.
    //
    // The fact that separates them is the ORACLE's borrow BASE: a `Borrowed`/`Join` whose
    // base may hold `v`'s store.  The `??` hoist temp carries a dep naming `v` —
    // `__lift_1` reads `["c", "__ref_p2_1"]` — because the hoist moved the READ and not
    // the VALUE.  A view of some OTHER binding has a base whose deps do not name `v`
    // (measured: `w`, `d`, `target` in `1184-a-view-assigned-back-onto-its-own-source`,
    // all with empty dep lists), and that population needs the in-place destination
    // precisely because it is assigning a view back onto its own source.
    let rhs_base = match crate::use_analysis::ownership_of(data, d_nr, value) {
        crate::use_analysis::Own::Borrowed { base } | crate::use_analysis::Own::Join { base } => {
            Some(base)
        }
        _ => None,
    };
    // A base of `u16::MAX` names no variable — a borrow of one of several arguments
    // (loft#1550) — so it may be `v` exactly when the value reads `v`.
    let rhs_may_alias_v = match rhs_base {
        Some(u16::MAX) => rhs_reads_v,
        Some(base) => base == v || function.tp(base).depend().contains(&v),
        None => false,
    };
    // loft#615 — an OWNED heap variable that is re-assigned must free the
    // store it is dropping, and `Vector` was missing from this list while
    // `Reference` / `Enum` had it.  A `??` materialises its subject into a
    // function-scope owner (`__ncc_N`, see
    // `parser/operators.rs::handle_null_coalesce`); re-assigning it once per
    // loop iteration overwrote the previous store with nothing freeing it, so
    // `for … { ns = list_dir(d) ?? [] }` leaked exactly (iterations - 1)
    // stores.  Empty deps is loft's owned-vs-borrowed convention, so a view
    // (`vv[i]`, a `??` result naming its owner) still takes no free here.
    // loft#723 — empty deps is only a PROXY for ownership, and it reads
    // "owned" for a borrow whose dep list was never populated.  A `??`
    // subject of Reference type is exactly that: the parser materialises
    // it into `__ncc_N` and marks it `skip_free` — the carried fact that
    // says its store belongs to someone else — while its type keeps empty
    // deps.  `skip_free`'s whole contract is "never emit `OpFreeRef` for
    // this var", so it must veto the proxy here rather than only at the
    // scope-exit sweep that `get_free_vars` runs.
    //
    // Consulting it only there left the unconditional pre-Set free below
    // reachable, and in a LOOP that free lands on the NEXT iteration's
    // store: `for … { e = f().items[0] ?? P {} }` freed the subject at the
    // body exit, `f()` reallocated the same slot, and the pre-Set free then
    // released that live store before the element read — silent stale bytes
    // without `LOFT_POISON`, SIGSEGV with it.  The @PLN118 sentinel reset
    // cannot cover it: that reset hangs off a block-exit free, which a
    // `skip_free` var by definition does not have.
    //
    // An OWNED Vector `??` subject is unaffected — the parser marks that one
    // `inline_ref`, not `skip_free`, precisely so it keeps the loft#615 free.
    //
    // The five KEYED kinds are here for the same reason `Vector` is, and were missing:
    // a keyed local's handle is the same store-backed slot, so `c = null` (which lowers
    // to `OpNullRefSentinel`, NOT to the @P302 in-place clear above) displaced its store
    // with nothing naming it — one orphan per evaluation on BOTH backends, values right
    // throughout.  Read through `base()`, because the shape that reaches this is the
    // NULLABLE one: a dense keyed local cannot be assigned the sentinel at all.
    //
    // `Vector` is read through `base()` for the same reason, and once did not be:
    // the bare spelling carried the claim that a nullable vector *"already releases
    // through its own path"*, so widening it *"would free twice"*.  Re-measured, that
    // is false — `x: vector<τ>? = null; for i in 0..N { x = m(i) }` over a fn-ref held
    // one store per iteration on BOTH backends, and the peak grew 1:1 with N while the
    // dense twin beside it stayed flat.  What makes the peel safe is `nullable_local`
    // below: an `Optional` destination routes to the runtime-GUARDED post-free, which
    // `free_displaced` no-ops on a same-store, free-protected or stack-record ref —
    // so the double free the bare spelling was avoiding cannot be reached from here.
    // Enforces @FR-O-Proxy, and is a SECOND home for the question
    // `Scopes::owns_freeable_store` already answers — *may this function free the
    // store this binding names?*  The last three conjuncts are that predicate
    // restated: `depend().is_empty() && !is_skip_free(v)` verbatim, and
    // `!is_hidden_buf_arg` where it spells the argument carve-out
    // `(!is_argument(v) || is_promoted_ret_buffer(..))`.  Two spellings of one rule
    // is what lets them disagree, and they do: the shape test in front of them does
    // not peel `Optional`, so a NULLABLE heap-record local (`c: S?` is
    // `Optional(Reference(S))`) matches no arm and skips this whole free path —
    // pre-free, stash and post-free alike — while its dense twin gets all three.
    // That is `formal/ownership.md` D-own-16's open half, and folding this onto
    // `owns_freeable_store` is where the peel belongs, so it lands once rather than
    // at whichever site a leak was noticed
    // (doc/claude/plans/own16-displaced-store-free/, step 2).
    //
    // ⚠ The peel is NOT sound on its own, which is why this comment is not a TODO to
    // widen the `matches!`: the empty dep list the rule above calls a PROXY reads
    // `OWNS` for a local a lambda has CAPTURED, and freeing there is a use-after-free
    // against the closure's capture-time DbRef.  The licence has to come from a
    // per-run witness.
    // @FR-O-Proxy asks free — the pre-Set free of the store `v` is displacing.  The
    // fact-reading half is `Function::owns_displaced_store`, the ONE spelling both
    // backends read (@FR-O-NoDiverge; its native twin is `generation/dispatch.rs`'s
    // `owned_ref_reassign`).  What this side adds is the interpreter's own: the hidden
    // buffer ARGUMENT is excluded, because the caller owns that store.
    // `@FR-O-Buffer` — a promoted return buffer may hold the store the CALLER handed,
    // which no free below may release: every one of them is guarded by the entry
    // witness the scope pass minted (native compares `_rb_w_<name>` at its rebind).
    let entry_w = function.entry_witness(v);
    // The hidden buffer argument is excluded only where no entry witness can tell the
    // caller's store from one this frame minted.  With one, the buffer is freed like any
    // owned local, guarded, as native's `owned_ref_reassign` does with `_rb_w_<name>`
    // (@FR-O-NoDiverge).  Excluding it outright left the store a frame minted for a
    // null-handed buffer to nobody when a call that took the buffer answered another
    // store: `none = rec(k - 1, none)` leaked one record per top-level call.
    // loft#2021 — a return temp bound in a branch written out per arm: its one dep
    // names the hidden return buffer, which the proxy reads as a borrow, while each
    // arm's bind copies into the temp (that is what the write-out is for), so the
    // temp owns what the arm displaces — the sentinel its pre-init put there.
    let owned_ref = (function.owns_displaced_store(v, value, data)
        || is_return_temp_owning_its_arms(data, d_nr, function, v))
        && (!is_hidden_buf_arg || entry_w.is_some());
    // An `OpNewRecord` RHS returns an INTERIOR ref into an existing
    // container's backing store (a vector element / nested field), so
    // the new value can land in the SAME store as v's old value —
    // consecutive elements of one vector share its backing.  The
    // unconditional pre-Set `OpFreeRef` below whole-store-frees that
    // backing, killing the live data: `xs[i].field = V{a: [literal]}`
    // (field-of-element set built in place) silently freed
    // `xs[i].field.a`'s store (correct bytes until churn overwrote the
    // freed slot; SIGSEGV/UAF under the armed build — the native
    // generator's `if _old.store_nr != new.store_nr` guard already
    // protects it).  Route it through the runtime-guarded post-free
    // (`OpFreeRefIfDistinct`) so a same-store reassignment is a no-op.
    let rhs_is_new_record = matches!(
        value.unspan(),
        Value::Call(fn_nr, _) if data.def(*fn_nr).name() == "OpNewRecord"
    );
    // A NULLABLE heap local's slot does not always hold an owned heap store: on the
    // paths where it never took one it holds the null sentinel, and where its record
    // was stack-allocated it holds a stack-record ref.  The pre-Set free below is
    // UNCONDITIONAL and whole-store, which is sound only for a local that always
    // holds a store of its own — a dense `Reference` does, an `Optional` one does
    // not.  Route it to the runtime-GUARDED post-free instead, which is the same
    // answer `rhs_is_new_record` above takes for the same reason: `free_displaced`
    // no-ops on a same-store, free-protected or stack-record displaced ref, where
    // `OpFreeRef` can only refuse it loudly (`#306`).  @FR-H-Free's `store(r) ≠ 0`
    // side condition is what separates the two frees; see `Stores::free_displaced`.
    let nullable_local = matches!(function.tp(v), Type::Optional(_));
    // loft#1336 / @FR-O-Witness — a local whose OWNER WITNESS releases its stores.
    // It is never-free, so `owned_ref` above is false and none of the frees below
    // are emitted for it; but the COPY arms further down must still run, because
    // @FR-B-Copy does not depend on who frees: a whole-value bind is independent.
    // Every copy into such a local lands in a FRESH store (the slot is reset before
    // the allocation), since the local may be holding a VIEW at that moment and an
    // in-place `OpDatabase` would write the copy into the viewed record.
    let witnessed = function.owner_witness(v).is_some();
    // loft#1522's veto (`rebind_keeps_displaced`) takes the displaced FREE away from an
    // owner that adopted a construction buffer's store — a struct literal built apart
    // because the local later borrows — and `owned_ref` above is false for it.  A VIEW
    // can carry the veto too (one path of `e = v[i]?` adopts a work ref) and is no owner:
    // its slot names the viewed record, which a copy arm would mint over.  It does
    // not take the COPY away: @FR-B-Copy does not depend on who frees, so the copy arms
    // below run for it as they do for a witnessed local, and add no free.  Without them
    // `x = S { … }; x = id(u)` aliased `u`, and the scope-exit free released `u`'s store
    // while `u` still named it — on the interpreter alone; the C backend copies at every
    // such bind.  A right-hand side that reads `x` keeps the plain bind: the callee may
    // answer `x`'s own store, which a copy arm would clear before copying from it.
    let buffer_kept = !rhs_reads_v
        && function.rebind_keeps_displaced(v)
        && function.owns_its_binding(v, value, data);
    // `@FR-O-Buffer` — the hidden buffer a right-hand-side CALL is handed may come
    // back as the call's result, and a POOLED buffer (`OpClear` on re-entry, one store
    // per site per activation) holds the previous call's record — which is what `v`
    // holds now.  Native frees the displaced store after the call and only when the new
    // value names another store (`owned_ref_reassign`); the unconditional pre-Set free
    // here released the pooled buffer under the callee, and the next pass wrote its
    // record into a freed store (measured: `x = a ?? mk(i)` in a loop,
    // `a-join-bound-local-owns-what-it-was-handed` jo2, once every exit literal built
    // into the buffer).  So the buffer is one more WITNESS of the displaced free below:
    // it keeps its position ahead of the call (the adopt pins read it there) and stops
    // firing exactly when the store it would release is the buffer's.
    let buffer_witness = function.displaced_buffer_witness(v, value, data);
    // `@FR-H-FreeTwice` — the compiler writes a null into one of its OWN temporaries only
    // after it released that temporary's store or handed it over: a free-source copy took it
    // (`take_local`), a consuming call did (`Scopes::scan_args`), or the temp's free just ran
    // (a displaced snapshot's reset, `scopes::drops`).  Its displaced store is gone, so the
    // reset frees nothing; a free here is the second release of that store.  A user's
    // `c = null` keeps its free: there the store the local held is the one being let go.
    let null_reset_of_temp = function.is_compiler_generated(v)
        && is_null_value(data, value)
        && !crate::keys::temp_null_reset_frees();
    let free = if null_reset_of_temp {
        DisplacedFree::None
    } else if owned_ref
        && (rhs_reads_v
            || rhs_is_new_record
            || nullable_local
            || rhs_reads_a_view_of(function, value, v))
    {
        DisplacedFree::Stash
    } else if owned_ref && !s1_substituted {
        let witness = if crate::keys::displaced_witness_enabled() {
            crate::use_analysis::projection_container_var(data, value)
        } else {
            None
        }
        .or(buffer_witness);
        DisplacedFree::Pre { witness }
    } else {
        DisplacedFree::None
    };
    let mut rb = ReBind {
        arm: ReArm::Plain,
        free,
        s1_substituted,
        is_hidden_buf_arg,
        rhs_reads_v,
        rhs_base,
        rhs_may_alias_v,
        entry_w,
        owned_ref,
        rhs_is_new_record,
        nullable_local,
        witnessed,
        buffer_kept,
        buffer_witness,
    };
    rb.arm = re_arm(data, d_nr, function, v, value, &rb);
    rb
}

/// The arm of a reassignment, given its facts: the tests in the order the old chain took them.
fn re_arm(
    data: &Data,
    d_nr: u32,
    function: &Function,
    v: u16,
    value: &Value,
    rb: &ReBind,
) -> ReArm {
    let tp = function.tp(v);
    if crate::parser::vectors::owns_keyed_store(tp) && *value == Value::Null {
        return ReArm::KeyedNull;
    }
    let stashed = rb.free == DisplacedFree::Stash;
    if (rb.owned_ref || rb.buffer_kept)
        && !rb.s1_substituted
        && !stashed
        && !matches!(value.unspan(), Value::Var(_))
        && crate::keys::join_own_enabled()
        && !function.is_view_elided(v)
        && let crate::use_analysis::Own::Join { base } =
            crate::use_analysis::ownership_of(data, d_nr, value)
        && base != u16::MAX
        && let Type::Reference(rec, _) = tp
    {
        return ReArm::Join { d_nr: *rec, base };
    }
    if !((rb.owned_ref || rb.witnessed || rb.buffer_kept) && !rb.s1_substituted) {
        return ReArm::Plain;
    }
    if rb.owned_ref
        && !rb.witnessed
        && !function.is_argument(v)
        && !function.is_view_elided(v)
        && let Some(rec) = crate::use_analysis::opaque_callref_bind(data, d_nr, tp, value)
    {
        return ReArm::OpaqueCallRef { rec };
    }
    if let Type::Reference(rec, _) | Type::Enum(rec, true, _) = tp.base().clone()
        && (!function.is_argument(v)
            // A renamed return buffer whose value reads it copies too where the copy lands in
            // a FRESH store (`copies_into_fresh`): an in-place copy would wipe the source it
            // reads, which is what excluded it, and bound plain it aliased whatever the call
            // answered — a view into another store its exit release then freed
            // (`@FR-H-FreeTwice`, O-Buffer's `class = pick(class, c)`).
            // `LOFT_NO_BUFFER_FRESH_COPY=1` keeps the plain bind.
            || (is_promoted_return_buffer(data, d_nr, function, v)
                && (!rb.rhs_reads_v
                    || (rb.copies_into_fresh() && !crate::keys::buffer_fresh_copy_off()))))
        && matches!(value.unspan(), Value::Call(_, _) | Value::CallRef(_, _))
        && let Some(fn_nr) = crate::use_analysis::callee_of(data, d_nr, value)
        && data.def(fn_nr).is_loft_defined()
        && (if crate::keys::reassign_copy_enabled() {
            !data.def(fn_nr).return_adopts_fresh_store()
        } else {
            data.def(fn_nr).attributes().iter().any(|a| {
                !a.hidden
                    && matches!(
                        a.typedef.base(),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    )
            })
        })
        && !(rb.rhs_reads_v
            && !rb.witnessed
            && data.def(fn_nr).returns_borrowed_view()
            && !(rb.rhs_base == Some(u16::MAX) && stashed)
            // The pass-through exists for an answer that may BE `v`'s own store (#360: a copy
            // would wipe the source it reads; and a recursion handing `v` back, `t = insert(t,
            // k)`, must not copy per call).  An answer that can never be `v`'s store
            // (`!rhs_may_alias_v`: `c = cond2(c, t, 3)` answering a mint or `t`) copies like any
            // other — bound plain, `c` aliased `t` and both freed it (`@FR-H-FreeTwice`).
            // `LOFT_NO_FRESH_COPY_OF_VIEW=1`.
            && !(!rb.rhs_may_alias_v && !crate::keys::fresh_copy_of_view_off()))
        && !function.is_view_elided(v)
        && !(function.is_deferred_first_bind(v)
            && crate::use_analysis::adopts_minted_at_bind(data, function, v, value))
    {
        return ReArm::CallCopy { d_nr: rec, fn_nr };
    }
    if let Some(rec) = tp.base().heap_def_nr()
        && let Value::Var(src) = value.unspan()
        && *src != v
        && let Some(src_d) = function.record_copy_source(data, v, *src)
        && data.copies_as(rec, src_d)
    {
        return ReArm::VarCopy {
            src: *src,
            d_nr: rec,
        };
    }
    let reads_v_on_exit_only = rb.rhs_reads_v && stashed && !reads_var_unless_exiting(value, v);
    if rb.owned_ref
        && let Some(rec) = tp.base().heap_def_nr()
        && (!rb.rhs_reads_v || reads_v_on_exit_only)
        && !rb.rhs_is_new_record
        && crate::use_analysis::projection_container_var(data, value).is_some()
    {
        return ReArm::ElementCopy { d_nr: rec };
    }
    // `@FR-B-Copy` — an owning record local rebound from a value that reads it and answers a
    // VIEW on some path (`x = x.next ?? y`: a stored reference's record, or another local):
    // copied into a fresh store with the value computed first (`R2d`), since bound plainly the
    // local named the other store and both freed it (`@FR-H-FreeTwice`).
    // `LOFT_NO_READING_VIEW_COPY=1` keeps the plain bind.
    if rb.owned_ref
        && stashed
        && rb.rhs_reads_v
        && !rb.rhs_is_new_record
        && !crate::keys::reading_view_copy_off()
        && let Some(rec) = tp.base().heap_def_nr()
        // A call's answer (bare, or the tail of the insert that hands it a literal:
        // `s = me(Bx { … })`) has its own delivery; a copy there orphans the handed store.
        && !matches!(
            delivered_tail(value),
            Value::Var(_) | Value::Call(_, _) | Value::CallRef(_, _)
        )
        && matches!(
            crate::use_analysis::ownership_of(data, d_nr, value),
            crate::use_analysis::Own::Borrowed { .. } | crate::use_analysis::Own::Join { .. }
        )
    {
        return ReArm::ElementCopy { d_nr: rec };
    }
    ReArm::Plain
}

/// Is `value` a null — `null`, or the sentinel a reference reset writes?
fn is_null_value(data: &Data, value: &Value) -> bool {
    match value.unspan() {
        Value::Null => true,
        Value::Call(d, args) if args.is_empty() => {
            matches!(
                data.def(*d).name(),
                "OpNullRefSentinel" | "OpConvRefFromNull"
            )
        }
        _ => false,
    }
}

/// The node a value delivers: the last operator through spans, inserts and blocks.
fn delivered_tail(value: &Value) -> &Value {
    match value.unspan() {
        Value::Insert(ops) => ops.last().map_or(value, delivered_tail),
        Value::Block(bl) => bl.operators.last().map_or(value, delivered_tail),
        v => v,
    }
}

/// Does a vector local's FIRST null init (`FirstArm::VectorNull`) mint a store of its own?  Not
/// for a local that borrows (`skip_free`, which also marks a match bind's or a `??`'s view), a
/// lift temporary overwritten before it is read (`inline_ref`), a lazy buffer that mints at its
/// first use (`@FR-O-LazyBuffer`), or a view (a dep).  The one answer the interpreter's code
/// generator (`gen_set_first_vector_null`) and the store records read.
#[must_use]
pub fn vector_null_mints(function: &Function, v: u16) -> bool {
    !(function.is_skip_free(v) || function.is_inline_ref(v) || function.is_lazy_buffer(v))
        && matches!(function.tp(v), Type::Vector(_, dep) if dep.is_empty())
}

/// Does a keyed local's FIRST null init (`FirstArm::KeyedNull`) mint a store of its own?  Not
/// for one that borrows an outer store (`inline_ref`) or a binding overwritten by its view
/// (`is_overwritten_view`, loft#1155); a never-free keyed local that hands its store on still
/// mints (`gen_keyed_null`).
#[must_use]
pub fn keyed_null_mints(function: &Function, v: u16) -> bool {
    !(function.is_inline_ref(v) || function.is_overwritten_view(v))
}

/// `@FR-O-LazyBuffer` — is `v = value` a lazy buffer's MINT?  Its entry null init wrote only the
/// sentinel (`vector_null_mints`), and the scope pass placed a later `Set(v, Null)` behind
/// `OpRefIsNull(v)`: that one makes the store.  Never `vector<T>?` (`lazy_buffer_mints`
/// declines those).  Read by the interpreter's code generator (`set_var`) and the store records.
#[must_use]
pub fn lazy_buffer_mints_at(function: &Function, v: u16, value: &Value) -> bool {
    matches!(value.unspan(), Value::Null)
        && function.is_lazy_buffer(v)
        && !matches!(function.tp(v), Type::Optional(_))
        && matches!(function.tp(v).base(), Type::Vector(_, dep) if dep.is_empty())
}
