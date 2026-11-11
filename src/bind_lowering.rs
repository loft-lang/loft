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

/// The lowering of the first assignment `v = value` in function `d_nr`.  The reasons for each
/// test live with the arm's emission in `state::codegen::gen_set_first_at_tos`.
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
