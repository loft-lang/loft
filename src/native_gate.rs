// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! @PLN11 Arc N / N2 — the **native-compilability gate**.
//!
//! In the native-library execution model (C71) a stable library compiles to
//! native code while the user script interprets, and calls dispatch across the
//! shared store ABI.  Not every function can be `--native`-compiled, so the
//! compiler partitions a library's functions into the **maximal native subgraph**
//! (compiled, dispatched via `OpStaticCall`) and the rest (run by the
//! interpreter).  This module computes that partition.
//!
//! **The denylist is small.**  Investigating the native backend (`generation/`)
//! found it emits *everything* — structs, enums, vectors, **generics, closures** —
//! except the concurrency constructs `parallel{}` / `par_for` / `yield`, for which
//! `emit.rs` writes a non-code comment.  (`NATIVE_SKIP` / `SCRIPTS_NATIVE_SKIP` in
//! `tests/native.rs` are both empty — the whole corpus compiles.)  So the
//! generics/closures "research problem" the plan feared was a phantom; the gate is
//! simply "uses no concurrency construct, transitively."
//!
//! **The gate is transitive and conservative** (Goal F — when unsure, interpret):
//! a function is native iff its body *and every function it `Call`s* are native.
//! This keeps the native subgraph **closed** — a native function only calls native
//! functions — so the runtime boundary is only ever *interpret → native*
//! (`OpStaticCall`, the supported direction); a native function never needs the
//! hard native → interpreter upcall.  A `CallRef` (runtime fn-ref) is excluded:
//! its dynamic callee can't be proven native.

use crate::data::{Data, DefType, Type, Value};
use std::collections::HashSet;

/// @PLN11 Arc N / N2 — the set of function `def_nr`s that can be `--native`-
/// compiled (the maximal native subgraph).  Everything not in the set runs
/// interpreted.  See the module docs for the gate's denylist + transitivity.
#[must_use]
pub fn native_compilable(data: &Data) -> HashSet<u32> {
    (0..data.definitions())
        .filter(|&d| {
            matches!(data.def(d).def_type(), DefType::Function)
                && fn_native_compilable(data, d, &mut HashSet::new())
        })
        .collect()
}

/// @PLN11 Arc N / N2 — the **scalar-dispatchable** subset: native-compilable
/// functions whose parameters and return type are all scalars
/// (`integer`/`boolean`/`float`/`single`/`character`).  These are the **first
/// dispatch slice**: no store reference crosses the interpreter↔native boundary,
/// so the auto-generated `#native` export wrapper is trivial and the
/// `&UnsafeCell<Stores>` ↔ `LoftStore` bridge is not needed (any internal store
/// use is contained — a scalar-return function cannot leak a store ref out).  The
/// store-touching functions (Text / Vector / Reference signatures) come in the
/// next slice with the bridge.
#[must_use]
pub fn scalar_dispatchable(data: &Data) -> HashSet<u32> {
    native_compilable(data)
        .into_iter()
        .filter(|&d| {
            let def = data.def(d);
            is_scalar_type(def.returned())
                && def
                    .attributes()
                    .iter()
                    // skip synthetic / hidden params (`__closure`, work buffers, …)
                    .filter(|a| !a.name.starts_with("__"))
                    .all(|a| is_scalar_type(&a.typedef))
        })
        .collect()
}

/// A type that passes the interpreter↔native boundary by value, with no store
/// reference — the trivially-marshallable set.  (Plain enums, `Text`, vectors and
/// references are deliberately excluded from this first slice.)  `Optional(τ)`
/// shares τ's sentinel layout (@PLN25), so it classifies as its base type.
fn is_scalar_type(t: &Type) -> bool {
    matches!(
        t.base(),
        Type::Integer(_) | Type::Boolean | Type::Float | Type::Single | Type::Character
    )
}

/// @PLN11 Arc N / N2 — the **shared-store-dispatchable** subset: native-compilable
/// functions that dispatch over the **shared `*mut Stores` bridge**
/// (`generate_shared_cdylib_lib_rs`): the caller's store is shared by pointer (no
/// `LoftStore` handle, no marshalling), so a non-scalar value can cross the
/// boundary.
///
/// **Parameters** may be any bridge type — scalars **plus** the `DbRef`-backed
/// aggregates (`vector`/`reference`/data-`enum`/sorted/index/hash/spatial), passed
/// as the raw stack `DbRef`.  **Returns** may be scalar, `void`, or a `vector`.  A
/// `vector` return triggers `--native`'s **hidden destination-parameter** convention
/// (`ref_return` appends a trailing `DbRef` work-vector marked `Attribute::hidden`
/// that the caller pre-allocates with `stores.null_named` + `OpDatabase(<type_id>)`);
/// the bridge wrapper allocates that destination itself, so the script-side
/// `#native` forward-decl still models only the public params.
///
/// Struct `reference` and data-`enum` returns are supported (no hidden dest — the
/// body allocates the record fresh).  Plain (tag-only) `enum` (a `u8` tag), `Text`
/// params (`&str`) and `Text` returns (via the `text_return` `&mut String` work
/// buffer the bridge owns) are supported.  Not yet handled: a `__closure` (or other
/// non-text-work `__`-prefixed) synthetic param (the function is then excluded).
#[must_use]
pub fn shared_store_dispatchable(data: &Data) -> HashSet<u32> {
    native_compilable(data)
        .into_iter()
        .filter(|&d| {
            let def = data.def(d);
            // `Optional(τ)` rides the same sentinel layout as τ (@PLN25) — gate on
            // the peeled type so a nullable return/param stays dispatchable.
            let ret = def.returned().base();
            // A vector return uses `--native`'s hidden destination param (the bridge
            // allocates it); a struct `reference` return does NOT (the body
            // allocates the record fresh and returns its `DbRef` —
            // `n_make_point(cell, a, b) -> DbRef`), so it needs no hidden dest.  A
            // `text` return uses `text_return`'s `&mut String` work buffer (the
            // bridge owns a local `String` and copies the result into scratch).
            let ret_text = matches!(ret, Type::Text(_));
            // Enum(_, _, _) covers both a plain (tag-only) enum (returned as a u8
            // tag) and a data enum (a DbRef, allocated fresh like a struct — no
            // hidden dest); `bridge_write_ret` distinguishes them.
            let ret_ok = matches!(ret, Type::Void | Type::Null)
                || is_scalar_type(ret)
                || matches!(
                    ret,
                    Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, _, _)
                )
                || ret_text;
            // loft#773 — a text return crosses the bridge THROUGH the `text_return`
            // work buffer, and the sentence above assumed every text-returning
            // function has one.  A function that returns a text it does not OWN — a
            // field read (`return self.nm`) or a literal (`return "fixed"`) — is
            // bufferless (`generation::def_returns_owned_text`), so the bridge holds
            // the bytes in a `String` local and has nowhere to put them: it dropped
            // them and the caller read `""`.  Dest-passing hid it wherever the result
            // was assigned to a local, so only using the call IN PLACE (an argument,
            // a format hole) showed it — silently, in the default mode of every
            // published library.  Such a function is not dispatchable; leaving it
            // unmarked makes it interpret, which is the same answer more slowly.
            let text_ret_has_buffer = !ret_text
                || def
                    .attributes()
                    .iter()
                    .any(|a| crate::native_lib::is_text_work_buffer(&a.typedef));
            ret_ok
                && text_ret_has_buffer
                && def
                    .attributes()
                    .iter()
                    .all(|a| classify_bridge_attr(a, ret_text).is_some())
        })
        .collect()
}

/// How a shared-bridge function's attribute is satisfied at the dispatch
/// boundary.  The interpreter call site pushes EVERY attribute onto the stack
/// (declaration order); the generated bridge reads only the `Marshal` ones
/// (compacted `LibArg` order) and satisfies the rest locally.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BridgeAttrKind {
    /// Visible parameter — popped by the dispatcher AND forwarded to the bridge.
    Marshal,
    /// `text_return` work buffer (`&text`) — popped but NOT forwarded (the
    /// bridge owns a local `String`).  In a non-dest call (expression context)
    /// the dispatcher reuses this caller-cleared cell as `bridge_text_dest`.
    WorkText,
    /// Hidden `ref_return` destination — popped but NOT forwarded (the bridge
    /// allocates its own destination in the shared store).
    HiddenDest,
}

/// #303 — the ONE marshallability judgment for the shared-store bridge.
/// Every consumer — this gate, the bridge generator
/// (`native_lib::shared_bridge_wrapper`), and the wire-time signature builder
/// (`extensions::compute_shared_sig`) — classifies an attribute through this
/// function.  `None` means the function is not shared-dispatchable.  Keeping
/// the judgment here is what guarantees a marked function always wires: the
/// gate marking it, the generator emitting its bridge, and the dispatcher
/// popping its stack slots can no longer disagree.
#[must_use]
pub fn classify_bridge_attr(a: &crate::data::Attribute, ret_text: bool) -> Option<BridgeAttrKind> {
    if a.hidden {
        // @P387: a text-returning fn's work-buffer is now `hidden` too (so it rides
        // the adaptive fn-ref dispatch like struct/vector return buffers).  It still
        // bridges as `WorkText` — the bridge owns a local `String` — NOT a heap dest.
        if crate::native_lib::is_text_work_buffer(&a.typedef) {
            return ret_text.then_some(BridgeAttrKind::WorkText);
        }
        // @PLAN59: every heap-kind hidden destination (Reference / Vector /
        // struct-Enum) is bridge-allocatable the same way — the wrapper's
        // HiddenDest arm resolves the store type via `hidden_dest_type_id`,
        // which already covers all three.  (Was Vector-only by accident.)
        return matches!(
            a.typedef.base(),
            Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, true, _)
        )
        .then_some(BridgeAttrKind::HiddenDest);
    }
    if crate::native_lib::is_text_work_buffer(&a.typedef) {
        // Valid iff the function actually returns text.
        return ret_text.then_some(BridgeAttrKind::WorkText);
    }
    if matches!(a.typedef.base(), Type::Function(..)) {
        return None; // closures — not handled (was a '__' name test)
    }
    is_bridge_type(&a.typedef).then_some(BridgeAttrKind::Marshal)
}

/// A type passable through the shared-store `LibArg` bridge **as a parameter**: a
/// scalar (by value), a `text` (as `&str` — ptr+len borrowed from the shared
/// store), or a `DbRef`-backed aggregate (`vector`/`reference`/data-`enum`/sorted/
/// index/hash/spatial), the latter shared by pointer through the caller's store.
/// (A `text` *return* is gated separately — it needs the `text_return` work
/// buffer, not just a slot.)
fn is_bridge_type(t: &Type) -> bool {
    // Enum(_, _, _): a plain (tag-only) enum (a u8 tag) or a data enum (a DbRef);
    // `bridge_read` distinguishes them.  Peel `Optional` — same layout as the base.
    is_scalar_type(t)
        || matches!(
            t.base(),
            Type::Text(_)
                | Type::Enum(_, _, _)
                | Type::Vector(_, _)
                | Type::Reference(_, _)
                | Type::Sorted(_, _, _)
                | Type::Index(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Radix(_, _, _)
                | Type::Trie(_, _, _)
        )
}

/// Transitive native-compilability of function `d_nr`: its body uses no
/// un-native-able construct AND every function it `Call`s is itself native.
/// `visited` breaks recursion cycles optimistically (a recursive self-call is
/// treated as native — the function's own body is checked on its first visit),
/// mirroring `scopes::walk_par_safe`.
fn fn_native_compilable(data: &Data, d_nr: u32, visited: &mut HashSet<u32>) -> bool {
    if d_nr == u32::MAX || d_nr >= data.definitions() {
        return false; // unknown / out-of-range callee → conservative
    }
    if !visited.insert(d_nr) {
        return true; // cycle — already being checked higher up the stack
    }
    let def = data.def(d_nr);
    if !matches!(def.def_type(), DefType::Function) {
        return false;
    }
    // @PLN24 — a `#c` definition is ALREADY bound, to a C symbol, and its empty
    // body is a declaration rather than compilable loft. Left in, the auto-native
    // driver claimed it: it generated a cdylib exporting `loft_shared_<name>`
    // bridges for symbols that live in a C library, overwrote `def.native` with
    // a bridge name, and every call then warned "could not be wired ... calling
    // it will panic". The binding is the implementation; nothing here may
    // replace it.
    if !def.c_sig.is_empty() {
        return false;
    }
    walk(def.code(), data, visited)
}

/// Exhaustive walk of a `Value` tree — **no `_` arm by design**: a future IR
/// variant must be classified here, so an un-native-able construct can never
/// silently slip through as "native" and produce broken Rust (Goal F:
/// conservative-silent, never a compile error in the user's face).  Returns
/// `false` the moment an un-native-able construct (or a non-native callee) is hit.
fn walk(v: &Value, data: &Data, visited: &mut HashSet<u32>) -> bool {
    match v {
        // The native backend cannot emit these (concurrency / coroutine) — see
        // `generation/emit.rs` (Parallel emits a non-code comment).
        Value::Parallel(_) | Value::Yield(_) => false,
        // Dynamic dispatch: the runtime callee is unknown → can't prove native.
        Value::CallRef(_, _) => false,
        // A static call: the callee must itself be native, and so must the args.
        Value::Call(callee, args) => {
            fn_native_compilable(data, *callee, visited)
                && args.iter().all(|a| walk(a, data, visited))
        }
        // Structural nodes carrying children — recurse into all of them.
        Value::Span(b) => walk(&b.1, data, visited),
        Value::Block(b) | Value::Loop(b) => b.operators.iter().all(|x| walk(x, data, visited)),
        Value::Insert(vs) | Value::Tuple(vs) => vs.iter().all(|x| walk(x, data, visited)),
        Value::Set(_, x) | Value::Return(x) | Value::Drop(x) | Value::TuplePut(_, _, x) => {
            walk(x, data, visited)
        }
        Value::If(c, t, e) => {
            walk(c, data, visited) && walk(t, data, visited) && walk(e, data, visited)
        }
        Value::Iter(_, create, next, init) => {
            walk(create, data, visited) && walk(next, data, visited) && walk(init, data, visited)
        }
        // Leaves — literals, var reads, fn-ref construction, keys, raw-expr.
        Value::Null
        | Value::Line(_)
        | Value::Int(_)
        | Value::Enum(_, _)
        | Value::Boolean(_)
        | Value::Float(_)
        | Value::Long(_)
        | Value::Single(_)
        | Value::Text(_)
        | Value::Var(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Keys(_)
        | Value::TupleGet(_, _)
        | Value::FnRef(_, _, _)
        | Value::FnRefDnr(_)
        | Value::RawExpr(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::boxed::Box as b; // terse `b::new(..)` for building test IR

    #[test]
    fn walk_classifies_leaves_and_denylist() {
        let data = Data::new();
        let mut seen = HashSet::new();
        // Leaves are native-OK.
        assert!(walk(&Value::Int(1), &data, &mut seen));
        assert!(walk(&Value::Text("x".into()), &data, &mut seen));
        // The concurrency / coroutine constructs are not.
        assert!(!walk(&Value::Parallel(vec![]), &data, &mut seen));
        assert!(!walk(&Value::Yield(b::new(Value::Null)), &data, &mut seen));
        // Dynamic dispatch is conservatively excluded.
        assert!(!walk(&Value::CallRef(0, vec![]), &data, &mut seen));
    }

    #[test]
    fn walk_finds_nested_denylist_construct() {
        let data = Data::new();
        let mut seen = HashSet::new();
        // A `parallel` buried in a branch must still flip the verdict — the walk
        // is exhaustive, so nesting cannot hide it.
        let nested = Value::If(
            b::new(Value::Boolean(true)),
            b::new(Value::Parallel(vec![Value::Int(1)])),
            b::new(Value::Null),
        );
        assert!(!walk(&nested, &data, &mut seen));
        // The same shape without the parallel arm is native.
        let clean = Value::If(
            b::new(Value::Boolean(true)),
            b::new(Value::Int(1)),
            b::new(Value::Null),
        );
        assert!(walk(&clean, &data, &mut HashSet::new()));
    }

    /// On the real stdlib the native backend compiles essentially everything, so
    /// the gate should classify the bulk of stdlib functions as native — and the
    /// total is non-trivial.  (Prints the coverage for visibility.)
    #[test]
    fn stdlib_is_mostly_native_compilable() {
        let mut p = crate::parser::Parser::new();
        if p.parse_dir("default", true, false).is_err() {
            eprintln!("skip: default/ stdlib not parseable here");
            return;
        }
        let total_fns = (0..p.data.definitions())
            .filter(|&d| matches!(p.data.def(d).def_type(), DefType::Function))
            .count();
        let native = native_compilable(&p.data);
        eprintln!(
            "native gate: {}/{} stdlib functions native-compilable",
            native.len(),
            total_fns
        );
        assert!(total_fns > 0, "stdlib should have functions");
        assert!(
            native.len() * 2 > total_fns,
            "most stdlib functions should be native-compilable ({}/{})",
            native.len(),
            total_fns
        );
    }

    /// The scalar-dispatchable subset (the first dispatch slice) is a non-empty
    /// subset of the native set — the stdlib has many scalar-signature functions.
    #[test]
    fn scalar_dispatchable_is_a_nonempty_subset() {
        let mut p = crate::parser::Parser::new();
        if p.parse_dir("default", true, false).is_err() {
            eprintln!("skip: default/ stdlib not parseable here");
            return;
        }
        let native = native_compilable(&p.data);
        let scalar = scalar_dispatchable(&p.data);
        eprintln!(
            "native gate: {}/{} native functions are scalar-dispatchable (first slice)",
            scalar.len(),
            native.len()
        );
        assert!(
            scalar.is_subset(&native),
            "scalar-dispatchable must be a subset of native-compilable"
        );
        assert!(
            !scalar.is_empty(),
            "stdlib should have scalar-only functions"
        );
        assert!(
            scalar.len() < native.len(),
            "not every native function is scalar-only (text/vector/ref signatures exist)"
        );
    }
}
