// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! Which IR makes a FRAME: the leaf rules (`@FR-R-LeafChain`) and the call-site line both
//! compiled targets record ask it.  Here, in `opt`, so the Rust and the C generators share
//! one answer without naming each other.

use crate::data::{Data, Value};

/// The compiler's store BRACKETS — the natives it emits around a call's reference arguments
/// (`set_store_lock` for a `const` parameter, `protect_store_frees` / `unprotect_store_frees`
/// for the fn-call free bracket).  Each sets or clears a flag on one store and calls no loft
/// code, so it neither re-enters a function nor reaches `stack_trace()`: a call to one is no
/// FRAME to the leaf rules (`@FR-R-LeafChain`), which a bracket added around an argument
/// would otherwise take a function out of.
#[must_use]
pub fn is_store_bracket(name: &str) -> bool {
    matches!(
        name,
        "n_set_store_lock" | "n_protect_store_frees" | "n_unprotect_store_frees"
    )
}

/// Does `code` make a call that can enter a frame — a user function, a method or free
/// overload with a loft body, a fn-ref, `parallel` or a `yield`?
///
/// The one answer to two questions: a function whose body makes none is a LEAF and carries
/// no frame (N4), and a statement that makes one first records the line it calls from
/// (`cr_call_site`, loft#1753).
#[must_use]
pub fn calls_a_frame(data: &Data, code: &Value) -> bool {
    code.any_node(&mut |v| match v {
        Value::Call(d, _) => {
            let callee = data.def(*d);
            let name = callee.name();
            if is_store_bracket(name) {
                return false;
            }
            // A free overload member (`f_…`) is a free function in all but its key.
            name.starts_with("n_")
                || callee.is_free_overload()
                || ((name.starts_with("t_") || callee.is_instance())
                    && matches!(callee.code(), Value::Block(_)))
        }
        Value::CallRef(..) | Value::Parallel(..) | Value::Yield(..) => true,
        _ => false,
    })
}
