// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! Which IR makes a FRAME: the call-site line a compiled target records asks it
//! (`@FR-R-GuardFree` asks the same of a function's call graph).  Here, in `opt`, beside the
//! analyses every backend reads.

use std::collections::{HashMap, HashSet};

use crate::data::{Data, Type, Value};

/// The compiler's store BRACKETS — the natives it emits around a call's reference arguments
/// (`set_store_lock` for a `const` parameter, `protect_store_frees` / `unprotect_store_frees`
/// for the fn-call free bracket).  Each sets or clears a flag on one store and calls no loft
/// code, so it neither re-enters a function nor reaches `stack_trace()`: a call to one is no
/// FRAME, which a bracket added around an argument would otherwise make of it.
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
/// A statement that makes one first records the line it calls from (`loft_frame_site`,
/// loft#1753).
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

/// `@FR-R-GuardFree`'s verdicts, per definition, for the unit being written.  The C generator
/// holds one: whether a frame needs the fn-ref buffer guard (the `loft_nfnrefs` mark and
/// `loft_fnref_release`).
#[derive(Default)]
pub struct GuardFree {
    free: HashMap<u32, bool>,
    registers: HashMap<u32, bool>,
    /// For a function that keeps its guard: the reachable callee that registers.
    witness: HashMap<u32, u32>,
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
    ///
    /// Answered for every function the question reaches at once, by its call graph's strongly
    /// connected components ([`call_sccs`], callees first): a component is guard-free when none
    /// of its members registers and every component it calls is.  Asked function by function
    /// over the closure, a call chain re-walked everything below each function (@PLN229 A3).
    pub fn is_guard_free(&mut self, data: &Data, def_nr: u32) -> bool {
        if let Some(&v) = self.free.get(&def_nr) {
            return v;
        }
        // Every loft-bodied callee, whatever its key: a generic INSTANCE (`i_1P_n_once`) is a
        // call like any other, and skipping it judged a frame guard-free above an instance
        // that registers buffers, which left the stores it handed up to nobody.
        let callees = |d: u32| -> Vec<u32> {
            let mut out: Vec<u32> = Vec::new();
            data.def(d).code().any_node(&mut |v| {
                if let Value::Call(c, _) = v
                    && matches!(data.def(*c).code(), Value::Block(_))
                    && !out.contains(c)
                {
                    out.push(*c);
                }
                false
            });
            out
        };
        let known = self.free.keys().copied().collect::<HashSet<u32>>();
        let trace = std::env::var("LOFT_TRACE_GUARD_FREE").is_ok();
        for scc in call_sccs(def_nr, &callees, &|d| known.contains(&d)) {
            // The member that registers, else the callee's witness that keeps it a guard.
            let mut witness = scc
                .iter()
                .copied()
                .find(|&d| self.registers_buffers(data, d));
            if witness.is_none() {
                witness = scc.iter().find_map(|&d| {
                    callees(d)
                        .into_iter()
                        .filter(|c| !scc.contains(c))
                        .find(|c| !self.free.get(c).copied().unwrap_or(false))
                        .map(|c| self.witness.get(&c).copied().unwrap_or(c))
                });
            }
            for &d in &scc {
                self.free.insert(d, witness.is_none());
                if let Some(w) = witness {
                    self.witness.insert(d, w);
                    if trace {
                        eprintln!(
                            "[guard-free] {}: keeps its guard — `{}` registers",
                            data.def(d).name(),
                            data.def(w).name()
                        );
                    }
                }
            }
        }
        self.free.get(&def_nr).copied().unwrap_or(false)
    }
}

/// The strongly connected components of the call graph reachable from `root` (`callees` names a
/// function's edges), in reverse topological order — every component after the ones it calls.
/// A function `known` answers already is a leaf: neither entered nor listed.  Tarjan's
/// algorithm, iterative, so a deep call chain needs no deep Rust stack.
pub(crate) fn call_sccs(
    root: u32,
    callees: &dyn Fn(u32) -> Vec<u32>,
    known: &dyn Fn(u32) -> bool,
) -> Vec<Vec<u32>> {
    let mut out: Vec<Vec<u32>> = Vec::new();
    if known(root) {
        return out;
    }
    let mut index: HashMap<u32, usize> = HashMap::new();
    let mut low: HashMap<u32, usize> = HashMap::new();
    let mut on_stack: HashSet<u32> = HashSet::new();
    let mut stack: Vec<u32> = Vec::new();
    let mut work: Vec<(u32, Vec<u32>, usize)> = Vec::new();
    let enter = |v: u32,
                 index: &mut HashMap<u32, usize>,
                 low: &mut HashMap<u32, usize>,
                 stack: &mut Vec<u32>,
                 on_stack: &mut HashSet<u32>,
                 work: &mut Vec<(u32, Vec<u32>, usize)>| {
        let n = index.len();
        index.insert(v, n);
        low.insert(v, n);
        stack.push(v);
        on_stack.insert(v);
        work.push((v, callees(v), 0));
    };
    enter(
        root,
        &mut index,
        &mut low,
        &mut stack,
        &mut on_stack,
        &mut work,
    );
    while let Some(top) = work.last_mut() {
        let v = top.0;
        if top.2 < top.1.len() {
            let w = top.1[top.2];
            top.2 += 1;
            if known(w) {
                continue;
            }
            if let Some(&wi) = index.get(&w) {
                if on_stack.contains(&w) {
                    let lv = low[&v].min(wi);
                    low.insert(v, lv);
                }
            } else {
                enter(
                    w,
                    &mut index,
                    &mut low,
                    &mut stack,
                    &mut on_stack,
                    &mut work,
                );
            }
            continue;
        }
        work.pop();
        if let Some(parent) = work.last() {
            let lp = low[&parent.0].min(low[&v]);
            low.insert(parent.0, lp);
        }
        if low[&v] == index[&v] {
            let mut scc = Vec::new();
            while let Some(w) = stack.pop() {
                on_stack.remove(&w);
                scc.push(w);
                if w == v {
                    break;
                }
            }
            out.push(scc);
        }
    }
    out
}
