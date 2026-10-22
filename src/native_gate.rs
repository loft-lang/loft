// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! @PLN11 Arc N / N2 — the **native-compilability gate**.
//!
//! In the native-library execution model (@C71) a stable library compiles to
//! native code while the user script interprets, and calls dispatch across the
//! shared store ABI.  Not every function can be `--native`-compiled, so the
//! compiler partitions a library's functions into the **maximal native subgraph**
//! (compiled, dispatched via `OpStaticCall`) and the rest (run by the
//! interpreter).  This module computes that partition.
//!
//! **The denylist is small.**  The native backend (`generation/`) emits everything —
//! structs, enums, vectors, generics, closures, `par` loops and `parallel { }` blocks
//! (`n_parallel_block_native`, loft#1054) and generators.  What a compiled LIBRARY still
//! cannot carry is a call through a function value, whose callee the cdylib cannot name,
//! and a generator whose yield has no native transport; @PLN199 removes those in turn.
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

/// Is `t` a `text_return` work buffer — a `&mut String` the inner fn appends the
/// result into (loft IR type `RefVar(Text)`)?  The bridge owns a local `String`
/// for each and passes `&mut`.  Shared with `native_lib` (the gate admits exactly
/// these as the one allowed `__`-prefixed param, and only for a text-returning fn).
pub(crate) fn is_text_work_buffer(t: &Type) -> bool {
    matches!(t, Type::RefVar(inner) if matches!(**inner, Type::Text(_)))
}

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
/// as the raw stack `DbRef`.  **Returns** may be scalar, `void`, a keyed collection
/// (a fresh store the caller owns, like a struct), or a `vector`.  A
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
        .filter(|&d| bridge_decline(data, d).is_none())
        .collect()
}

/// Why a function the gate admits cannot cross the shared-store bridge, in words, or `None`
/// when its signature can — the judgment [`shared_store_dispatchable`] filters on.
#[must_use]
pub fn bridge_decline(data: &Data, d: u32) -> Option<String> {
    {
        {
            let def = data.def(d);
            // `Optional(τ)` rides the same sentinel layout as τ (@PLN25) — gate on
            // the peeled type so a nullable return/param stays dispatchable.
            let ret = def.returned().base();
            // A vector return uses `--native`'s hidden destination param (the bridge
            // allocates it); a struct `reference` return does NOT (the body
            // allocates the record fresh and returns its `DbRef` —
            // `n_make_point(cell, a, b) -> DbRef`), so it needs no hidden dest.  A
            // `text` return is written into the destination the caller hands over
            // (`bridge_text_dest`): a local it is assigned to, or the temporary the
            // parser gives a call in value position (`Parser::crosses_text_bridge`,
            // which also covers a function returning a text it does not own — loft#773).
            let ret_text = matches!(ret, Type::Text(_));
            // Enum(_, _, _) covers both a plain (tag-only) enum (returned as a u8
            // tag) and a data enum (a DbRef, allocated fresh like a struct — no
            // hidden dest); `bridge_write_ret` distinguishes them.
            let ret_ok = matches!(ret, Type::Void | Type::Null)
                || is_scalar_type(ret)
                || matches!(
                    ret,
                    Type::Vector(_, _)
                        | Type::Reference(_, _)
                        | Type::Enum(_, _, _)
                        // @PLN199 A1 — a keyed collection is returned like a struct: the
                        // body allocates a fresh store and answers its `DbRef`, with no
                        // hidden destination, and the caller owns it.
                        | Type::Sorted(_, _, _)
                        | Type::Index(_, _, _)
                        | Type::Hash(_, _, _)
                        | Type::Radix(_, _, _)
                        | Type::Trie(_, _, _)
                )
                // @PLN199 D1 — a generator: the bridge registers it in the host's coroutine
                // table and answers the handle (`HostTable::coroutine_adopt`).
                || matches!(ret, Type::Iterator(elem, _) if generator_crosses(elem))
                // @PLN199 C1 — a function value is returned as its `(d_nr, closure)`; the
                // interpreter calls it like any of its own.
                || matches!(ret, Type::Function(..))
                || ret_text;
            if !ret_ok {
                return Some(format!(
                    "it returns `{}`, which the bridge cannot carry",
                    ret.name(data)
                ));
            }
            def.attributes()
                .iter()
                .find(|a| classify_bridge_attr(a, ret_text).is_none())
                .map(|a| {
                    format!(
                        "its parameter `{}: {}` cannot cross the bridge",
                        a.name,
                        a.typedef.name(data)
                    )
                })
        }
    }
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
        if is_text_work_buffer(&a.typedef) {
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
    if is_text_work_buffer(&a.typedef) {
        // Valid iff the function actually returns text.
        return ret_text.then_some(BridgeAttrKind::WorkText);
    }
    if matches!(a.typedef.base(), Type::Function(..)) {
        // @PLN199 C1 — a function value crosses as its `(d_nr, closure)`; calling it from the
        // cdylib needs the call back for a callee the cdylib does not carry.
        return crate::fnref::upcall_carries(&a.typedef).then_some(BridgeAttrKind::Marshal);
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
    // @PLN199 A3 — a tuple crosses as its leaves, one `LibArg` each.
    if let Type::Tuple(elems) = t.base() {
        return elems.iter().all(is_tuple_leaf);
    }
    // @PLN199 A4 — a `&T` to a scalar crosses as the `DbRef` of the caller's slot; the bridge
    // hands the inner fn a pointer into it (`*mut i64`), the form `--native` passes.
    if let Type::RefVar(inner) = t.base() {
        return is_ref_scalar(inner);
    }
    // @PLN199 D3 — a generator handle, interpreted or compiled: both advance through the
    // host's table (`codegen_runtime::CoroutineHost`).
    if let Type::Iterator(elem, _) = t.base() {
        return generator_crosses(elem);
    }
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

/// @PLN199 D1 — may a script iterate a library generator yielding `t`?  The interpreter
/// advances a compiled generator through the host's table, reading the yield on the channel
/// `--native`'s consumer reads (`State::native_coroutine_next`): a full-width integer, a
/// float, a boolean, or a store reference.  A text, a tuple, a function value and the narrow
/// scalars each want their own read.
pub(crate) fn generator_crosses(t: &Type) -> bool {
    match t.base() {
        Type::Integer(s) => s.forced_size.is_none(),
        Type::Float | Type::Boolean | Type::Text(_) => true,
        t => crate::data::is_dbref(t),
    }
}

/// @PLN199 A4 — may `&t` cross the bridge?  A full-width integer, a float or a boolean: the
/// slot the `DbRef` names holds exactly the `i64` / `f64` / `u8` the inner fn's `*mut` reads
/// and writes.  `&text`, `&vector` and `&(…)` each want their own pointer form, and a narrow
/// integer, a `single` or a `character` a slot read at its own width.
pub(crate) fn is_ref_scalar(t: &Type) -> bool {
    match t.base() {
        Type::Integer(s) => s.forced_size.is_none(),
        Type::Float | Type::Boolean => true,
        _ => false,
    }
}

/// @PLN199 A3 — may `t` be an element of a tuple PARAMETER?  The dispatcher pops a tuple
/// leaf by leaf, each with the step [`crate::database::Stores::get`] pops for its type, and
/// every element sits in its own 8-rounded slot (`data::element_stack_offsets_in`).  Those
/// agree for a full-width integer, a float, a boolean, a text and a store reference; a
/// `single`, a `character` and a plain enum are narrower in the slot than the dispatcher's
/// read, and stay out until it reads them at their own width.  A nested tuple flattens.
pub(crate) fn is_tuple_leaf(t: &Type) -> bool {
    match t.base() {
        Type::Tuple(elems) => elems.iter().all(is_tuple_leaf),
        Type::Integer(s) => s.forced_size.is_none(),
        Type::Float
        | Type::Boolean
        | Type::Text(_)
        | Type::Enum(_, true, _)
        | Type::Vector(_, _)
        | Type::Reference(_, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _)
        | Type::Trie(_, _, _) => true,
        _ => false,
    }
}

/// Transitive native-compilability of function `d_nr`: its body uses no
/// un-native-able construct AND every function it `Call`s is itself native.
/// `visited` breaks recursion cycles optimistically (a recursive self-call is
/// treated as native — the function's own body is checked on its first visit),
/// mirroring `scopes::walk_par_safe`.
fn fn_native_compilable(data: &Data, d_nr: u32, visited: &mut HashSet<u32>) -> bool {
    fn_decline(data, d_nr, visited).is_none()
}

/// Why a function stays interpreted — the first construct the gate met that a compiled
/// library cannot carry.  `None` means it compiles.  The `bool` gate above is this with the
/// reason dropped, so the trace and the decision cannot disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decline {
    /// Not a function (or out of range): nothing to compile.
    NotFunction,
    /// A `#c` binding: the C symbol is the implementation.
    CBinding,
    /// A generator whose yield type has no native transport (`coroutine_layout::CHANNEL_NONE`).
    Yield,
    /// The body calls through a function value, whose callee cannot be proven compiled.
    CallRef,
    /// The body calls this function, which itself stays interpreted.
    Callee(u32),
}

fn fn_decline(data: &Data, d_nr: u32, visited: &mut HashSet<u32>) -> Option<Decline> {
    if d_nr == u32::MAX || d_nr >= data.definitions() {
        return Some(Decline::NotFunction); // unknown / out-of-range callee → conservative
    }
    if !visited.insert(d_nr) {
        return None; // cycle — already being checked higher up the stack
    }
    let def = data.def(d_nr);
    if !matches!(def.def_type(), DefType::Function) {
        return Some(Decline::NotFunction);
    }
    // @PLN24 — a `#c` definition is ALREADY bound, to a C symbol, and its empty
    // body is a declaration rather than compilable loft. Left in, the auto-native
    // driver claimed it: it generated a cdylib exporting `loft_shared_<name>`
    // bridges for symbols that live in a C library, overwrote `def.native` with
    // a bridge name, and every call then warned "could not be wired ... calling
    // it will panic". The binding is the implementation; nothing here may
    // replace it.
    if !def.c_sig.is_empty() {
        return Some(Decline::CBinding);
    }
    // @PLN199 D1 — a generator compiles, unless its yield has no native transport: the
    // generator's emitter answers that with a `compile_error!` naming it, which in a library
    // would fail the whole cdylib.
    if let Type::Iterator(elem, _) = def.returned().base()
        && crate::coroutine_layout::channel_tag(elem) == crate::coroutine_layout::CHANNEL_NONE
    {
        return Some(Decline::Yield);
    }
    walk_decline(def.code(), d_nr, data, visited)
}

/// [`walk_decline`] as the yes/no the gate asks (the unit tests' spelling).
#[cfg(test)]
fn walk(v: &Value, data: &Data, visited: &mut HashSet<u32>) -> bool {
    walk_decline(v, u32::MAX, data, visited).is_none()
}

/// Exhaustive walk of a `Value` tree — **no `_` arm by design**: a future IR
/// variant must be classified here, so an un-native-able construct can never
/// silently slip through as "native" and produce broken Rust (Goal F:
/// conservative-silent, never a compile error in the user's face).  Answers the first
/// un-native-able construct (or non-native callee) met, `None` when there is none.
fn walk_decline(v: &Value, owner: u32, data: &Data, visited: &mut HashSet<u32>) -> Option<Decline> {
    let all = |vs: &[Value], visited: &mut HashSet<u32>| {
        vs.iter()
            .find_map(|x| walk_decline(x, owner, data, visited))
    };
    match v {
        // A generator body compiles (@PLN199 D1); its frame is registered in the host's
        // coroutine table when it crosses the bridge.
        Value::Yield(x) => walk_decline(x, owner, data, visited),
        // A call through a function value: the arms the cdylib carries run compiled, and any
        // other callee runs in the interpreter that loaded it, through the call back
        // (@PLN199 C2) — for the signatures that call back carries.
        Value::CallRef(v_nr, args) => {
            let carried = owner < data.definitions()
                && crate::fnref::upcall_carries(data.def(owner).variables().tp(*v_nr));
            if carried {
                all(args, visited)
            } else {
                Some(Decline::CallRef)
            }
        }
        // A static call: the callee must itself be native, and so must the args.
        // @PLN199 B2 — a `#c` callee is: the cdylib calls its C symbol through the bind
        // table every generated library carries (`__C_LIBS` / `__C_LIB_SYMS`, @PLN197), as
        // a whole-program build does.  Only the `#c` DEFINITION stays out of the export set
        // (`fn_decline`'s `CBinding`): the binding is its implementation.
        Value::Call(callee, args) => {
            let c_binding = *callee < data.definitions() && !data.def(*callee).c_sig.is_empty();
            (if c_binding {
                None
            } else {
                fn_decline(data, *callee, visited)
            })
            .map(|_| Decline::Callee(*callee))
            .or_else(|| all(args, visited))
        }
        // Structural nodes carrying children — recurse into all of them.
        Value::Span(b) => walk_decline(&b.1, owner, data, visited),
        Value::Block(b) | Value::Loop(b) => all(&b.operators, visited),
        // @PLN199 B1 — a `parallel { }` block compiles (`n_parallel_block_native`); its
        // arms are walked like any other code.
        Value::Insert(vs) | Value::Tuple(vs) | Value::Parallel(vs) => all(vs, visited),
        Value::Set(_, x) | Value::Return(x) | Value::Drop(x) | Value::TuplePut(_, _, x) => {
            walk_decline(x, owner, data, visited)
        }
        Value::If(c, t, e) => walk_decline(c, owner, data, visited)
            .or_else(|| walk_decline(t, owner, data, visited))
            .or_else(|| walk_decline(e, owner, data, visited)),
        Value::Iter(_, create, next, init) => walk_decline(create, owner, data, visited)
            .or_else(|| walk_decline(next, owner, data, visited))
            .or_else(|| walk_decline(init, owner, data, visited)),
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
        | Value::RawExpr(_) => None,
    }
}

/// Why function `d_nr` stays INTERPRETED when its library compiles, in words, or `None`
/// when it is compiled and dispatched over the shared-store bridge.  The gate's reason
/// first, followed down a chain of interpreted callees to the construct at its root; then
/// the bridge's, for a function the gate admits but whose signature cannot cross.  What
/// `LOFT_TRACE_NATIVE_GATE=1` prints for each export of a `use`d library.
#[must_use]
pub fn explain_interpreted(data: &Data, d_nr: u32) -> Option<String> {
    let mut chain = Vec::new();
    let mut at = d_nr;
    loop {
        match fn_decline(data, at, &mut HashSet::new()) {
            None => break,
            Some(Decline::Callee(c)) if chain.len() < 8 && c != at => {
                chain.push(data.def(c).name().to_string());
                at = c;
            }
            Some(d) => {
                let why = match d {
                    Decline::NotFunction => "an unknown callee".to_string(),
                    Decline::CBinding => {
                        "a `#c` binding (the C symbol is the implementation)".to_string()
                    }
                    Decline::Yield => "a generator whose yield has no compiled form".to_string(),
                    Decline::CallRef => {
                        "a call through a function value of a signature the interpreter \
                         cannot be called back with"
                            .to_string()
                    }
                    Decline::Callee(c) => format!("a cycle through `{}`", data.def(c).name()),
                };
                return Some(if chain.is_empty() {
                    format!("its body uses {why}")
                } else {
                    format!("it calls `{}`, which uses {why}", chain.join("` → `"))
                });
            }
        }
    }
    bridge_decline(data, d_nr)
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
        // A `parallel` block of leaves compiles, and so does a generator body.
        assert!(walk(
            &Value::Parallel(vec![Value::Int(1)]),
            &data,
            &mut seen
        ));
        assert!(walk(&Value::Yield(b::new(Value::Null)), &data, &mut seen));
        // Dynamic dispatch is conservatively excluded.
        assert!(!walk(&Value::CallRef(0, vec![]), &data, &mut seen));
    }

    #[test]
    fn walk_finds_nested_denylist_construct() {
        let data = Data::new();
        let mut seen = HashSet::new();
        // A function-value call buried in a branch, inside a `parallel` arm, must still
        // flip the verdict — the walk is exhaustive, so nesting cannot hide it.
        let nested = Value::If(
            b::new(Value::Boolean(true)),
            b::new(Value::Parallel(vec![Value::CallRef(0, vec![])])),
            b::new(Value::Null),
        );
        assert!(!walk(&nested, &data, &mut seen));
        // The same shape with a leaf in the arm is native.
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
