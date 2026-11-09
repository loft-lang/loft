// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! Which IR makes a FRAME: the leaf rules (`@FR-R-LeafChain`) and the call-site line both
//! compiled targets record ask it.  Here, in `opt`, so the Rust and the C generators share
//! one answer without naming each other.

use std::collections::{HashMap, HashSet};

use crate::data::{Data, Type, Value};

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

/// `@FR-R-KeyedEntry`'s probe (`keyed_entry_ints` / `_text`), the IR's form of a keyed
/// insert: a store call that finds or files one entry and calls no loft code — no FRAME, as
/// the claim and file it replaces were none.
#[must_use]
pub fn is_store_entry(name: &str) -> bool {
    matches!(name, "n_keyed_entry_ints" | "n_keyed_entry_text")
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
            if is_store_bracket(name) || is_store_entry(name) {
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

/// `@FR-R-GuardFree`'s verdicts, per definition, for the unit being written.  The Rust and the
/// C generators each hold one and read the same answer: whether a frame needs the fn-ref
/// buffer guard (`codegen_runtime::FnRefBufGuard`, the C's `loft_nfnrefs` mark and
/// `loft_fnref_release`).
#[derive(Default)]
pub struct GuardFree {
    free: HashMap<u32, bool>,
    registers: HashMap<u32, bool>,
}

impl GuardFree {
    /// Does this definition's OWN body register a store against the running frame's fn-ref
    /// buffer list, or run something whose registrations are not visible here?  The pushers
    /// are the fn-ref dispatch (native `cr_fnref_buf` / `cr_fnref_minted`, C `loft_fnref_buf`
    /// / `loft_fnref_mint`, all under a `CallRef`) and native's op `OpFreeRefOrHandUp`; a
    /// `parallel` body and a `yield` are opaque and count as registering, and so is a callee
    /// with NO loft body that is not the standard library's (a user's native function) or
    /// that takes a fn-ref (it may call back into user code).  A `#rust`-bodied stdlib
    /// function is the body's own work.  Every other node registers nothing: the list is
    /// pushed only by the dispatch of a call through a function value.
    pub fn registers_buffers(&mut self, data: &Data, def_nr: u32) -> bool {
        if let Some(&v) = self.registers.get(&def_nr) {
            return v;
        }
        let registers = data.def(def_nr).code().any_node(&mut |v| match v {
            Value::Call(d, _) => {
                let callee = data.def(*d);
                let name = callee.name();
                let user_fn = name.starts_with("n_") || name.starts_with("t_");
                let opaque = user_fn
                    && !matches!(callee.code(), Value::Block(_))
                    && (!crate::file_access::is_stdlib_source(callee.position.file.as_str())
                        || callee
                            .attributes()
                            .iter()
                            .any(|a| matches!(a.typedef.base(), Type::Function(..))));
                name == "OpFreeRefOrHandUp" || opaque
            }
            Value::CallRef(..) | Value::Parallel(..) | Value::Yield(..) => true,
            _ => false,
        });
        self.registers.insert(def_nr, registers);
        registers
    }

    /// Is NOTHING that registers a fn-ref buffer reachable from this definition — over the
    /// closure of its call graph, cycles included?  Then no entry can ever stand above the
    /// mark its buffer guard would take, its release is empty every time, and the frame
    /// carries no guard.  A cycle does not block: the guard never depended on acyclicity.
    /// Decides `@FR-R-GuardFree`.
    pub fn is_guard_free(&mut self, data: &Data, def_nr: u32) -> bool {
        if let Some(&v) = self.free.get(&def_nr) {
            return v;
        }
        let mut seen: HashSet<u32> = HashSet::new();
        let mut todo: Vec<u32> = vec![def_nr];
        let mut free = true;
        while let Some(d) = todo.pop() {
            if !seen.insert(d) {
                continue;
            }
            if self.registers_buffers(data, d) {
                if std::env::var("LOFT_TRACE_GUARD_FREE").is_ok() {
                    eprintln!(
                        "[guard-free] {}: keeps its guard — `{}` registers",
                        data.def(def_nr).name(),
                        data.def(d).name()
                    );
                }
                free = false;
                break;
            }
            data.def(d).code().any_node(&mut |v| {
                if let Value::Call(c, _) = v {
                    // Every loft-bodied callee, whatever its key: a generic INSTANCE
                    // (`i_1P_n_once`) is a call like any other, and skipping it judged a
                    // frame guard-free above an instance that registers buffers, which
                    // left the stores it handed up to nobody.
                    if matches!(data.def(*c).code(), Value::Block(_)) && !seen.contains(c) {
                        todo.push(*c);
                    }
                }
                false
            });
        }
        self.free.insert(def_nr, free);
        free
    }
}
