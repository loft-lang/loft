// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! The program's functions as a compiled target names them: which are reachable from the
//! entry points, and the one identifier each is emitted under.  The identifier is a contract
//! between the code that writes a compiled library and the loader that finds its symbols, so
//! both ask here.

use crate::data::{Data, DefType, Type, Value};
use crate::data_store::ValueType;
use crate::fnref;
use crate::ir_node::IrNode;
use std::collections::{HashMap, HashSet, VecDeque};

/// Mark every function `node` can reach.  Recursion is delegated to
/// [`IrNode::for_each_child`] so the walk is TOTAL by construction: only the
/// node kinds needing extra *extraction* (a worker fn-nr riding as an integer
/// literal) appear below, never the tree shape itself.  A per-kind whitelist
/// here silently pruned the callee of a `Tuple` element and left rustc to fail
/// E0425 on the emitted call (loft#815).
pub(crate) fn collect_calls(node: IrNode, data: &Data, calls: &mut HashSet<u32>) {
    if node.kind() == ValueType::Call {
        let d = node.call_to();
        let args = node.call_args();
        calls.insert(d);
        // The parallel family passes its worker function as an integer literal that the
        // codegen emitter (src/generation/ops/parallel.rs) resolves into a closure body
        // calling the worker by name.  Detect it here so the worker is included in the
        // reachable set — without this, the closure refers to a fn that never gets
        // emitted and rustc fails with "cannot find function" (E0425).  WHICH argument is
        // the one table `fnref::parallel_worker_arg`, shared with the value-record gate.
        if let Some((i, min)) = fnref::parallel_worker_arg(data.def(d).name())
            && args.len() >= min
            && args.get(i).kind() == ValueType::Int
            && args.get(i).int_value() >= 0
        {
            calls.insert(args.get(i).int_value() as u32);
        }
    }
    node.for_each_child(&mut |c| collect_calls(c, data, calls));
}

/// Recursively collect all `Int` literals from a value tree that may represent
/// fn-ref constants (e.g. inside `if`/`block` branches of a function-typed `Set`).
///
/// Callers pass a tree already known to PRODUCE a fn-ref, so every value
/// position in it is a candidate — recursion is delegated to
/// [`IrNode::for_each_child`] to keep the walk total (a fn-ref literal reaches
/// the same places any other value does: a `Tuple` element, a `break` value, a
/// nested `Set`).  `Call` is the one deliberate exception: there the fn-ref is
/// the call's RESULT, and its ARGUMENTS are ordinary integers that must not be
/// mistaken for def numbers.
/// Mark the functions a tuple literal reaches through its `fn(…)` members, at any DEPTH.
///
/// The flat version of this was loft#1069: a function reachable only through a tuple was
/// pruned as unreachable, then had no arm in the native fn-ref dispatch and the call
/// panicked `invalid fn-ref`.  It read the TOP-level members only, so a fn one level in
/// (`((dbl, 1), "z")`) was pruned exactly as before — nesting depth being the axis that
/// fix held fixed.
///
/// Read per MEMBER where the shapes line up, so only a member the destination declares
/// `fn(…)` contributes; a tuple whose arity does not match falls back to scanning the
/// whole value, which over-approximates exactly as @P299 does.  Reachability
/// over-approximation is correctness-safe — it can only emit an unused candidate.
pub(crate) fn collect_tuple_fn_refs(elems: &[Type], inner: &Value, calls: &mut HashSet<u32>) {
    let Value::Tuple(items) = inner.unspan() else {
        collect_int_fn_refs(IrNode::Native(inner), calls);
        return;
    };
    if items.len() != elems.len() {
        collect_int_fn_refs(IrNode::Native(inner), calls);
        return;
    }
    for (e, item) in elems.iter().zip(items.iter()) {
        match e.base() {
            Type::Function(..) => collect_int_fn_refs(IrNode::Native(item), calls),
            Type::Tuple(nested) if nested.iter().any(crate::data::tuple_carries_fn_ref) => {
                collect_tuple_fn_refs(nested, item, calls);
            }
            _ => {}
        }
    }
}

pub(crate) fn collect_int_fn_refs(node: IrNode, calls: &mut HashSet<u32>) {
    match node.kind() {
        ValueType::Int => {
            let n = node.int_value();
            if n >= 0 {
                calls.insert(n as u32);
            }
        }
        // FnRef(d_nr, clos_var, _) is used for closure fn-refs.
        ValueType::FnRef => {
            let d = node.fnref_dnr();
            if d >= 0 {
                calls.insert(d.cast_unsigned());
            }
        }
        // See the doc comment: a call's args are values, not def numbers.
        ValueType::Call | ValueType::CallRef => {}
        _ => node.for_each_child(&mut |c| collect_int_fn_refs(c, calls)),
    }
}

/// Scan a definition's code for fn-ref literals:
/// - `Set(var, Int(n))` where `var` has a `Function` or `Routine` type
/// - `Call(d, args)` where a parameter of `d` is `Function`/`Routine` typed and the
///   corresponding arg is `Int(n)`
///
/// These are function-pointer uses like `f = fn double_it` or `apply_fn(fn double_it, x)`.
pub(crate) fn collect_fn_ref_literals(
    val: &Value,
    data: &Data,
    variables: &crate::variables::Function,
    calls: &mut HashSet<u32>,
    // #263: true when the enclosing definition's return type is a fn-ref
    // (`Function`/`Routine`).  A fn-ref returned as a bare d_nr (`return dbl`
    // → `Value::Return(Value::Int(d_nr))`) is otherwise invisible to this
    // walk — the `Return` arm recurses into a plain `Int`, which hits the
    // `_ => {}` no-op — so the returned lambda is pruned as unreachable and
    // the caller's `f(x)` dispatch panics with `invalid fn-ref`.  Same shape
    // as the @P299 (`OpSetInt4(field, …)`) and @P328 (`yield`) recoveries.
    returns_fn: bool,
) {
    match val {
        Value::Set(var, inner) => {
            // Through the LINK as well as bare: `@FR-B-Ref-Intro` gives `&τ` for every τ, so
            // a `&fn(…)` parameter names a fn-ref slot and a d_nr written through it keeps
            // that function reachable.  Read bare, a function reached ONLY by a write
            // through such a link was pruned, had no arm in the native dispatch, and the
            // call panicked `invalid fn-ref` — the same shape as the @P299, @P328 and
            // loft#1069 recoveries above, with the link as the spelling that hid it
            // (loft#1443).
            // `peel_link` IS this question — `base()` and then every `&` — and asking it by
            // name rather than by a local `match` is `@FR-B-Ref-Uniform`'s own argument: the
            // linkage lives in the TYPE, so a site that wants to know what a value IS says so
            // once.  Hand-rolled here and at four more sites across loft#1443/#1454/#1455
            // before the accessor reached this tree.
            let slot_tp = variables.tp(*var).peel_link();
            if matches!(slot_tp, Type::Function(..) | Type::Routine(_)) {
                collect_int_fn_refs(IrNode::Native(inner), calls);
            }
            // loft#1069 — a fn-ref stored into a TUPLE MEMBER. `t: (fn(…), integer) =
            // (dbl, 1)` puts the d_nr in a plain `Int` element of a `Value::Tuple`, and
            // the variable's type is a `Tuple`, not a `Function` — so neither the arm
            // above nor the FnRef-literal walk sees it, and a function called ONLY through
            // a tuple was pruned as unreachable. It then had no arm in the native fn-ref
            // dispatch and the call panicked `invalid fn-ref`. Same shape as the @P299
            // (`OpSetInt4` into a struct field) and @P328 (`yield`) recoveries.
            //
            // Read per MEMBER where the shapes line up, so only a member the destination
            // declares `fn(…)` contributes; a tuple whose arity does not match falls back
            // to scanning the whole value, which over-approximates exactly as @P299 does
            // (an integer member equal to a fn d_nr marks that fn reachable). Reachability
            // over-approximation is correctness-safe — it can only emit an unused
            // candidate.
            if let Type::Tuple(elems) = variables.tp(*var)
                && elems.iter().any(crate::data::tuple_carries_fn_ref)
            {
                collect_tuple_fn_refs(elems, inner, calls);
            }
            collect_fn_ref_literals(inner, data, variables, calls, returns_fn);
        }
        // The ASSIGNMENT half of the tuple case above: `t.0 = inc` writes the d_nr into a
        // member without ever building a `Value::Tuple`, so the walk that reads a tuple
        // LITERAL per member never sees it and a function reachable only this way was
        // pruned — `invalid fn-ref: 711` at the call.  Here the destination names one
        // member exactly, so the element type is read straight rather than zipped.
        Value::TuplePut(var, idx, inner) => {
            if let Type::Tuple(elems) = variables.tp(*var)
                && elems
                    .get(*idx as usize)
                    .is_some_and(|e| matches!(e.base(), Type::Function(..)))
            {
                collect_int_fn_refs(IrNode::Native(inner), calls);
            }
            collect_fn_ref_literals(inner, data, variables, calls, returns_fn);
        }
        Value::Call(d, args) => {
            let callee = data.def(*d);
            // @P299 — a fn-ref stored into a struct FIELD lowers to
            // `OpSetInt4(target, pos, Int(<lambda d_nr>))`: the lambda's d_nr
            // rides as a plain Int (capturing closures wrap this in a
            // `fn_ref_field_set` block, non-capturing emit it bare — see
            // `parser/mod.rs::set_field_check`).  The FnRef-literal walk below
            // can't see it, so a closure called ONLY through a struct field is
            // pruned as unreachable and dropped from the native fn-ref dispatch
            // candidate set (`emit.rs::output_call_ref`) → `invalid fn-ref`
            // panic.  Recover it: if `OpSetInt4` writes a literal that is a
            // valid Function definition, mark it reachable.  This
            // over-approximates (a plain int field equal to a fn d_nr would
            // mark that fn reachable too) but reachability over-approximation
            // is correctness-safe — it only ever emits an unused candidate.
            // The fused append (@PLN157 § V-m) writes the same d_nr as `OpPushInt4(vector,
            // Int(<lambda d_nr>))` — a `vector<fn(…)>` literal's elements — one operand
            // earlier.
            let fn_ref_operand = match callee.name.as_str() {
                "OpSetInt4" => args.get(2),
                "OpPushInt4" => args.get(1),
                _ => None,
            };
            if let Some(arg2) = fn_ref_operand
                && let Value::Int(dn) = arg2.unspan()
                && *dn >= 0
                && (*dn as u32) < data.definitions()
                && matches!(data.def(*dn as u32).def_type(), DefType::Function)
                && !data.def(*dn as u32).name().starts_with("Op")
            {
                calls.insert(*dn as u32);
            }
            for (idx, a) in args.iter().enumerate() {
                if idx < callee.attributes.len()
                    && matches!(
                        callee.attributes[idx].typedef,
                        Type::Function(..) | Type::Routine(_)
                    )
                {
                    collect_int_fn_refs(IrNode::Native(a), calls);
                }
            }
        }
        // The fn-ref CALLEE half of the `Value::Call` arm above, and the last shape in this
        // list to be given one.  A call THROUGH a fn-ref is `Value::CallRef(v_nr, args)`, so
        // the arm that reads a NAMED callee's declared parameter types never saw it, and a
        // lambda passed as an argument to a fn-ref callee — `lf(fn(x: integer) -> integer
        // { x * 2 })` — was pruned as unreachable.  It then had no arm in the native fn-ref
        // dispatch and the call panicked `invalid fn-ref: <d_nr>` (loft#1285).
        //
        // The parameter types come from the fn-ref VARIABLE's own type, which is the only
        // place a lambda callee declares them.  Over-approximation is correctness-safe here
        // exactly as it is for @P299 and loft#1069 — it can only emit an unused candidate.
        Value::CallRef(v_nr, args) => {
            if let Type::Function(param_types, ..) = variables.tp(*v_nr) {
                for (idx, a) in args.iter().enumerate() {
                    if idx < param_types.len()
                        && matches!(
                            param_types[idx].base(),
                            Type::Function(..) | Type::Routine(_)
                        )
                    {
                        collect_int_fn_refs(IrNode::Native(a), calls);
                    }
                }
            }
        }
        Value::Return(v) => {
            // #263: a fn-ref-returning fn whose return value is a bare d_nr
            // (`return dbl` → `Int(d_nr)`) — pick the Int up as a reachable
            // fn-ref literal.  `collect_int_fn_refs` recurses through any
            // block/if wrapping the return value too.
            if returns_fn {
                collect_int_fn_refs(IrNode::Native(v), calls);
            }
        }
        // FnRef inside a Block result (closure allocation block).
        Value::FnRef(d_nr, _, _) if *d_nr >= 0 => {
            calls.insert((*d_nr).cast_unsigned());
        }
        // @P328 — `yield <fn-ref>` for a non-capturing closure emits as
        // `Value::Yield(Value::Int(d_nr))` (the parser drops the closure
        // wrapper when there's nothing to capture).  Treat the inner Int
        // as a fn-ref literal so the lambda stays reachable for native
        // CallRef dispatch — without this the loop-body call `f(x)`
        // panics at runtime with `invalid fn-ref: <d_nr>`.  Same shape
        // as the @P299 fix for `OpSetInt4(field, pos, Int(d_nr))`;
        // over-approximation is correctness-safe.
        Value::Yield(inner) => collect_int_fn_refs(IrNode::Native(inner), calls),
        _ => {}
    }
    // Recursion is delegated to the keystone so this walk stays total: only the
    // node kinds above carry extra EXTRACTION, never the tree shape (loft#815).
    val.for_each_child(&mut |c| collect_fn_ref_literals(c, data, variables, calls, returns_fn));
}

/// Compute the set of function definitions reachable from `entry_defs` via
/// transitive calls and fn-ref literals.  Returns the full reachable set
/// including `entry_defs`.
#[must_use]
pub fn reachable_functions(data: &Data, entry_defs: &[u32]) -> HashSet<u32> {
    let mut reachable = HashSet::new();
    let mut queue: VecDeque<u32> = entry_defs.iter().copied().collect();
    // @PLN167 C3 (loft#1656) — a function's STORE instances (`<key>@st<mask>`) are reachable
    // with it: a call through a function value reaches an instance only through its dispatch
    // arm, which names the function it copies.  Over-approximation, correctness-safe as the
    // fn-ref rules below are: it can only emit an unused instance.
    let mut instances: HashMap<(String, u16), Vec<u32>> = HashMap::new();
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.is_store_text_instance()
            && let Some((base, _)) = def.name().split_once('@')
        {
            instances
                .entry((base.to_string(), def.source()))
                .or_default()
                .push(d);
        }
    }
    while let Some(d) = queue.pop_front() {
        if !reachable.insert(d) {
            continue;
        }
        let def = data.def(d);
        let mut calls = HashSet::new();
        collect_calls(IrNode::Native(def.code()), data, &mut calls);
        // #263: a fn-ref returned as a bare d_nr is only a fn-ref literal when
        // this def's return type IS a fn-ref — otherwise an ordinary integer
        // return would be misread as a reachable fn d_nr.
        let returns_fn = matches!(def.returned(), Type::Function(..) | Type::Routine(_));
        collect_fn_ref_literals(def.code(), data, def.variables(), &mut calls, returns_fn);
        // A reachable lambda's closure record can be released through a fn-ref that no local
        // record names, and `OpDropFnRef` dispatches to its cascade by `d_nr` (loft#1609).
        let record = def.closure_record();
        if record != u32::MAX {
            let cascade = data.drop_cascade_nr(record);
            if cascade != u32::MAX {
                calls.insert(cascade);
            }
        }
        if let Some(list) = instances.get(&(def.name().to_string(), def.source())) {
            calls.extend(list.iter().copied());
        }
        for c in calls {
            if !reachable.contains(&c) {
                queue.push_back(c);
            }
        }
    }
    reachable
}

/// Function names carried by MORE than one Function/Dynamic def.  Two modules
/// may export the same `pub fn` name — `Data` scopes defs by `(name, source)`,
/// but emitted Rust is one flat namespace, so such names need a disambiguated
/// identifier (#305: rustc E0428 "defined multiple times").
///
/// # Panics
/// When two DIFFERENT keys flatten to one Rust identifier — a compiler defect, since the
/// emitted crate would define one symbol twice.
#[must_use]
pub fn duplicate_fn_names(data: &Data) -> HashSet<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut dups = HashSet::new();
    // Two DIFFERENT keys that flatten to one Rust identifier ([`rust_fn_ident`]) would be two
    // definitions under one symbol — a key keeps its readable characters (`i_15vector<text>_…`,
    // `f_10Rock#Paper_…`) and this is the one place that makes it an identifier, so it is the
    // one place that can refuse the collision.  Two definitions of the SAME key from two
    // modules are the ordinary duplicate, disambiguated below.
    let mut flat: HashMap<String, &str> = HashMap::new();
    for d in 0..data.definitions() {
        let def = data.def(d);
        if !matches!(def.def_type(), DefType::Function | DefType::Dynamic) {
            continue;
        }
        if !seen.insert(def.name()) {
            dups.insert(def.name().to_string());
        }
        let ident = rust_fn_ident(def.name());
        if let Some(prev) = flat.insert(ident.clone(), def.name()) {
            assert!(
                prev == def.name(),
                "native emission: the definitions `{prev}` and `{}` flatten to one Rust \
                 identifier `{ident}`",
                def.name()
            );
        }
    }
    dups
}

/// The Rust identifier emitted for function `def`.  Unique names keep their
/// bare ident (stable existing output); a name in `dups` gets a short content
/// hash of its defining FILE appended — two same-named fns can only come from
/// different files (the parser rejects an in-file redefinition), so the pair
/// (name, defining file) is unique program-wide.
///
/// That last sentence is a DEPENDENCY on the loader, not a property of this
/// function: it holds only while one file is parsed into one source.  When a
/// module reachable under two names was parsed twice, both copies hashed the
/// same path, every duplicated function was emitted under one identifier, and
/// the cdylib would not compile — 55 × `error[E0428] … defined multiple times`,
/// each pair carrying an IDENTICAL hash, which is what distinguishes it from
/// #305's two genuinely different files.  The guard is
/// `Parser::source_loaded_from`, which is keyed on the canonical path for the
/// same reason this is (loft#1080).
#[must_use]
pub fn disambiguated_fn_ident(dups: &HashSet<String>, def: &crate::data::Definition) -> String {
    let name = def.name();
    let base = if dups.contains(name) {
        // FNV-1a over the defining file path — deterministic across runs.
        let mut h: u32 = 0x811c_9dc5;
        for b in def.position().file.bytes() {
            h ^= u32::from(b);
            h = h.wrapping_mul(0x0100_0193);
        }
        format!("{name}_m{h:08x}")
    } else {
        name.to_string()
    };
    rust_fn_ident(&base)
}

/// Flatten a loft def name into a valid Rust identifier.  Most names already are
/// (`n_foo`, `t_4Pair_first`), but a generic instantiated over a TUPLE carries the
/// synthetic tuple struct's schema name verbatim — `t_24__tuple<integer,integer>_first`
/// — and `<`, `>`, `,`, and spaces are not valid in a Rust identifier (#395).  Every
/// emission of a fn name (definition AND every call) routes through `fn_ident`, so
/// flattening at this one chokepoint keeps the definition and its callers in sync.
pub(crate) fn rust_fn_ident(name: &str) -> String {
    if name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return name.to_string();
    }
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
