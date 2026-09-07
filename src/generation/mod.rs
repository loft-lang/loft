// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust code generator (--native)

use crate::data::{Context, Data, DefType, Type, Value};
use crate::data_store::ValueType;
use crate::database::Stores;
use crate::ir_node::{IrBlock, IrNode};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
mod calls;
mod coroutine;
mod dispatch;
mod emit;
pub mod hoist;
pub(crate) mod ops;
mod pre_eval;
mod text;

/// One hoisted binding produced by `collect_pre_evals`:
/// `(name, match_code, bind_code, counter, replace_all)` — `name` is the
/// `_pre_N` temp, `match_code` is the node's regenerated text (legacy
/// string-recogniser fallback), `bind_code` is what the `let` binds (may
/// differ from `match_code`, e.g. a narrow-int `as i64` wrap), `counter` is the
/// codegen counter when generated, `replace_all` substitutes every occurrence
/// (template dup-param).  Stored in `PreEvalSet`, which adds intrinsic
/// node-identity keys — see `pre_eval.rs`.
type PreEvalEntry = (String, String, String, u32, bool);

/// Rust source spliced into the native `main` bootstrap just before its
/// closing brace.  Gated on `LOFT_NATIVE_LEAK_CHECK` so normal native
/// runs stay silent, but tests (and a curious developer) can opt in to
/// the same store-leak report the interpreter prints unconditionally —
/// giving leak regressions a guard on `--native`, not just `--interpret`.
const NATIVE_LEAK_CHECK_TAIL: &str = "    if std::env::var(\"LOFT_NATIVE_LEAK_CHECK\").is_ok() {\n        let stores: &Stores = unsafe { &*cell.get() };\n        let leaks = stores.collect_store_leaks();\n        if !leaks.is_empty() {\n            let count = leaks.len();\n            let preview = if count <= 5 { leaks.join(\", \") } else { format!(\"{} ... and {} more\", leaks[..5].join(\", \"), count - 5) };\n            eprintln!(\"Warning: {count} stores not freed at program exit: {preview}\");\n        }\n    }\n";

/// @PLN130 F8 — the `LOFT_STRICT_STORES` tail for the native `main` bootstrap.
///
/// Native needs its own copy because the generated binary never runs `src/main.rs`, so the
/// interpreter's exit check cannot see it. Without this the reports printed during the run
/// but the process still exited 0 — a gate that reports and passes is not a gate.
///
/// It covers BOTH halves: the use-after-free count accumulated during the run, and the
/// stores nobody freed. Native is the backend where the lifetime faults actually happen
/// (the interpreter reuses a store in place instead of freeing it), so this is the arm that
/// matters most.
/// Host-only (`#[cfg(not(target_arch = "wasm32"))]`, the same gate the store-timeline tail
/// uses). Two reasons: strict mode is a probe diagnostic run on a developer's machine, and
/// the wasm targets link a SEPARATELY built `libloft` rlib — so a tail referencing a symbol
/// added to `loft::keys` fails to compile there whenever that rlib is stale, which is a
/// build break in the generated program rather than anything a probe wanted to know.
const NATIVE_STRICT_STORE_TAIL: &str = "    #[cfg(not(target_arch = \"wasm32\"))]\n    if std::env::var(\"LOFT_STRICT_STORES\").is_ok() {\n        let stores: &Stores = unsafe { &*cell.get() };\n        let leaks = stores.collect_store_leaks();\n        if !leaks.is_empty() {\n            eprintln!(\"[strict-store] NEVER FREED: {} stores not freed at program exit: {}\", leaks.len(), leaks.join(\", \"));\n            loft::keys::strict_store_leaks(leaks.len());\n        }\n        let n = loft::keys::strict_store_violations();\n        if n > 0 {\n            eprintln!(\"[strict-store] FAILED: {n} store-lifetime violation(s)\");\n            std::process::exit(1);\n        }\n    }\n";

/// Walk the Value IR tree and collect all function definition numbers
/// referenced by `Value::Call(def_nr, _)` nodes.
/// Detect a T-parameterized method stub: name shape
/// `t_<digits><identifier>_<method>` where `<identifier>` is the
/// generic type variable's name.  These stubs are synthesized by
/// `parse_function`'s I7/I8.1 path for every interface method on a
/// bound type parameter (`fn foo<T: Bound>(...)`).  At call sites
/// they are substituted with the concrete impl via
/// `re_resolve_call`, so the stub body is never entered at runtime.
///
/// The discriminator from a regular `t_<LEN><Type>_<method>` (e.g.
/// `t_4text_starts_with`) is that `<Type>` here is a known concrete
/// type defined in the program; for T-stubs it's a generic type
/// variable name.  We can't distinguish those at this layer without
/// access to the Data table, so we accept the false-positive
/// possibility (a real builtin matching the same shape but missing
/// from `codegen_runtime.rs`) — those would emit `todo!()` instead
/// of `compile_error!()`, which still aborts at runtime if reached
/// (just at the call site rather than at compile time).
///
/// Pragmatic bar: any `t_<digits><alpha-prefix>_*` whose body is
/// empty + no `#rust` + no `#native` is treated as a T-stub.  The
/// caller already checks the empty-body branch.
///
/// Plan-12 phase 1a (2026-05-23) — `is_crypto_runtime_symbol` removed.
/// Crypto symbols now live in `lib/crypto/native/` (declared via
/// `lib/crypto/loft.toml::native = "loft_crypto"`); they route through
/// the standard package native path instead of a hardcoded list in the
/// compiler crate.
fn is_t_param_stub(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("t_") else {
        return false;
    };
    // Parse the leading length digits.
    let len_end = rest.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(0);
    if len_end == 0 {
        return false;
    }
    let Ok(type_len) = rest[..len_end].parse::<usize>() else {
        return false;
    };
    let after_len = &rest[len_end..];
    if after_len.len() < type_len + 1 {
        return false;
    }
    // The next `type_len` chars are the type name; then `_` then method.
    let type_name = &after_len[..type_len];
    let after_type = &after_len[type_len..];
    if !after_type.starts_with('_') {
        return false;
    }
    // Heuristic: a generic type variable is a single ASCII identifier
    // (mostly UPPERCASE single letter or short PascalCase).  Concrete
    // builtins use lowercase type names (`text`, `integer`, `single`,
    // `float`, `boolean`, `character`, `enum`, `function`, plus narrow
    // variants and user struct/enum names which are CamelCase).  Treat
    // a type-name component starting with an uppercase letter as a
    // potential generic-T stub.  This catches `T`, `T_p205`, `U`,
    // `MyTrait` etc.
    //
    // The known concrete CamelCase types (`JsonValue`, `JsonField`,
    // `RegexCapture`, etc.) are wired in `codegen_runtime.rs` already
    // and never reach this branch (the `is_codegen_runtime_fn` check
    // earlier returns true for them).  Any concrete CamelCase type
    // that ISN'T wired AND has an empty body would be incorrectly
    // labeled "T-stub" — but in that case the right behaviour is
    // identical (emit `todo!()` instead of compile_error since the
    // function is genuinely unimplemented).
    type_name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
}

/// Mark every function `node` can reach.  Recursion is delegated to
/// [`IrNode::for_each_child`] so the walk is TOTAL by construction: only the
/// node kinds needing extra *extraction* (a worker fn-nr riding as an integer
/// literal) appear below, never the tree shape itself.  A per-kind whitelist
/// here silently pruned the callee of a `Tuple` element and left rustc to fail
/// E0425 on the emitted call (loft#815).
fn collect_calls(node: IrNode, data: &Data, calls: &mut HashSet<u32>) {
    if node.kind() == ValueType::Call {
        let d = node.call_to();
        let args = node.call_args();
        calls.insert(d);
        // n_parallel_for / n_parallel_queue pass a worker function as
        // args[4]: an integer literal that the codegen emitter
        // (src/generation/ops/parallel.rs) resolves into a closure body
        // calling the worker by name.  Detect it here so the worker
        // is included in the reachable set — without this, the
        // closure refers to a fn that never gets emitted and rustc
        // fails with "cannot find function" (E0425).
        if matches!(
            data.def(d).name(),
            "n_parallel_for"
                | "n_parallel_for_light"
                | "n_parallel_queue"
                | "n_parallel_queue_text"
                | "n_parallel_queue_ref"
                | "n_parallel_queue_narrow"
                | "n_parallel_queue_fn"
                | "n_parallel_discard"
        ) && args.len() >= 5
            && args.get(4).kind() == ValueType::Int
            && args.get(4).int_value() >= 0
        {
            calls.insert(args.get(4).int_value() as u32);
        }
        // ARC.md A5b — par_fold uses a different arg layout than
        // the for/queue family: the worker fn d_nr is at args[2]
        // (after input + init).  Same reason for the insert: the
        // ParallelFoldEmitter generates `worker_name(cell, acc, row)`
        // and the worker must be in the reachable set.
        if data.def(d).name() == "n_parallel_fold"
            && args.len() >= 4
            && args.get(2).kind() == ValueType::Int
            && args.get(2).int_value() >= 0
        {
            calls.insert(args.get(2).int_value() as u32);
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
fn collect_tuple_fn_refs(elems: &[Type], inner: &Value, calls: &mut HashSet<u32>) {
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
            Type::Function(_, _, _) => collect_int_fn_refs(IrNode::Native(item), calls),
            Type::Tuple(nested) if nested.iter().any(crate::data::tuple_carries_fn_ref) => {
                collect_tuple_fn_refs(nested, item, calls);
            }
            _ => {}
        }
    }
}

fn collect_int_fn_refs(node: IrNode, calls: &mut HashSet<u32>) {
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
fn collect_fn_ref_literals(
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
            if matches!(
                variables.tp(*var),
                Type::Function(_, _, _) | Type::Routine(_)
            ) {
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
                    .is_some_and(|e| matches!(e.base(), Type::Function(_, _, _)))
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
            if callee.name == "OpSetInt4"
                && let Some(arg2) = args.get(2)
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
                        Type::Function(_, _, _) | Type::Routine(_)
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
            if let Type::Function(param_types, _, _) = variables.tp(*v_nr) {
                for (idx, a) in args.iter().enumerate() {
                    if idx < param_types.len()
                        && matches!(
                            param_types[idx].base(),
                            Type::Function(_, _, _) | Type::Routine(_)
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
        let returns_fn = matches!(def.returned(), Type::Function(_, _, _) | Type::Routine(_));
        collect_fn_ref_literals(def.code(), data, def.variables(), &mut calls, returns_fn);
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
#[must_use]
pub fn duplicate_fn_names(data: &Data) -> HashSet<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut dups = HashSet::new();
    for d in 0..data.definitions() {
        let def = data.def(d);
        if !matches!(def.def_type(), DefType::Function | DefType::Dynamic) {
            continue;
        }
        if !seen.insert(def.name()) {
            dups.insert(def.name().to_string());
        }
    }
    dups
}

/// Final invariant scrub over a COMPLETE generated native source: rewrite the
/// loft-internal module references `crate::rpc::` / `crate::store::` to
/// `loft::…`.  In the interpreter's own crate `crate == loft`, so a `#rust`
/// template can write `crate::rpc::print_or_capture` / `crate::store::Store`;
/// but in a GENERATED binary `crate::` is the binary's own root (no `rpc` /
/// `store` module), so they must be `loft::…` (loft is linked as an extern
/// crate; both fns are `pub`).  The host-import intrinsics
/// `crate::loft_host_print` / `crate::wasm::…` are the generated cdylib's OWN
/// items and correctly stay `crate::` — only the two specific module paths are
/// rewritten.
///
/// This is the CHOKEPOINT: every native-emission entry scrubs its full output
/// here, so it cannot matter which template/inlining path produced a
/// `crate::rpc::` reference.  A per-call-site rewrite in
/// `substitute_template_body` missed an emission path and produced an
/// intermittent `error[E0433]: cannot find rpc in crate` in the nightly
/// index-hygiene leg — enforcing the invariant once, over the assembled
/// source, is robust where the per-site rewrite was not.
fn scrub_generated_crate_refs(src: &[u8]) -> Vec<u8> {
    let s = String::from_utf8_lossy(src);
    if !s.contains("crate::rpc::") && !s.contains("crate::store::") && !s.contains("crate::state::")
    {
        return src.to_vec();
    }
    s.replace("crate::rpc::", "loft::rpc::")
        .replace("crate::store::", "loft::store::")
        .replace("crate::state::", "loft::state::")
        .into_bytes()
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
fn rust_fn_ident(name: &str) -> String {
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

/// Use this to drive Rust code generation from a compiled loft program.
/// It bundles the read-only compile-time data with the mutable emission state
/// so that individual emits functions don't need to pass both separately.
#[allow(clippy::struct_excessive_bools)]
pub struct Output<'a> {
    pub data: &'a Data,
    pub stores: &'a Stores,
    pub counter: u32,
    pub def_nr: u32,
    pub indent: u32,
    pub declared: HashSet<u16>,
    /// loft#1371 — locals this function bound as a `&struct` link through `OpCreateStack`,
    /// which native represents as a `*mut DbRef` into the source's slot so a whole-value
    /// write reaches the source (`@FR-B-Ref-Write`).  A `RefVar` local that merely ALIASES
    /// another `RefVar` (the #257 shape) is not one of these — it copies the pointer it was
    /// given — so the read side asks this set rather than the type alone.
    pub local_record_link: HashSet<u16>,
    /// Hidden return-buffer (retbuf) attribute vars that have an entry-buffer
    /// witness `_rb_w_<name>` emitted in the prologue (capturing the caller's
    /// buffer at function entry).  A CONDITIONAL reassignment of such a
    /// return-local frees an orphaned fn-owned intermediate guarded by
    /// `_old != _rb_w_<name>`, so it never frees the caller's buffer — closing
    /// the cluster-462 native record leak without an over-free.
    pub retbuf_witness: HashSet<u16>,
    /// @PLN90 #495 — "runtime-Join" locals: an owned-typed Reference/Enum local
    /// that is INITIALISED owned (a whole-value copy / owned call) but then
    /// REASSIGNED to a borrow (the `r = v[i] ?? x` ncc) at least once.  r's
    /// runtime ownership then differs per path (owned copy on the empty-loop
    /// path, a borrowed view once the reassign runs), so BOTH the in-loop
    /// displaced-free and the scope-exit `OpFreeRef(r)` — which key on r's owned
    /// TYPE — would whole-store-free a caller-owned view.  For each such var a
    /// prologue `_own_store_<name>: DbRef` tracks the store r actually OWNS (NULL
    /// once r holds a borrow); the two free-sites free `_own_store_<name>`, never
    /// the view.  Scoped to exactly one owned assign + ≥1 ncc-borrow reassign.
    pub witness_vars: HashSet<u16>,
    /// #260 Fix B: `__vdb` store locals declared up front in the function
    /// prologue (sentinel-bound, no allocation).  The first body
    /// `Set(v, Null)` consumes its entry so it still emits the named-store
    /// `null_named` + `OpDatabase` pair at its (possibly reclaim-relocated)
    /// IR position — allocation order and store naming stay unchanged;
    /// only the `let` position moved.
    pub predeclared: HashSet<u16>,
    /// Active hoisted sub-expressions for the statement being emitted: IR node
    /// address → its `_pre_N` name.  `output_code_inner` consults this at every
    /// node so a hoisted operand emits its name instead of being re-generated
    /// inline — the single source of truth for pre-eval identity (no counter
    /// re-derivation, no regenerate-and-string-match).  Saved/restored per
    /// statement in `output_block`; empty outside pre-eval emission.  See
    /// COMPILER.md § Synthesised-identity stability.
    pub active_pre_eval: HashMap<usize, String>,
    /// Set of reachable `def_nrs` for native output (populated by `output_native_reachable`).
    pub reachable: HashSet<u32>,
    /// Function names defined by MORE than one def (two modules may export the
    /// same `pub fn` — scoped by source in `Data`, but flat in emitted Rust).
    /// Populated at the generation entries; [`Output::fn_ident`] consults it so
    /// collision members get a disambiguated identifier (#305: E0428).
    pub dup_fn_names: HashSet<String>,
    /// Stack of enclosing loop scope ids, innermost last.
    /// Used to emit Rust labeled breaks for `Value::Break(n)` with n > 0.
    pub loop_stack: Vec<u16>,
    /// loft#885 — vector headers hoisted out of the enclosing loops, innermost last:
    /// variable → the Rust local holding its [`crate::vector::VecHeader`]. An indexed read
    /// of a variable listed here skips the store resolution and the length load; every
    /// other read emits unchanged. One frame is pushed per `Value::Loop`, so a frame is
    /// popped exactly where the local it names goes out of scope.
    pub vec_headers: Vec<HashMap<u16, String>>,
    /// Per-definition memo behind [`hoist::may_write_store`], shared across every loop in
    /// the program so the call-graph walk runs once per callee.
    pub hoist_cache: HashMap<u32, bool>,
    /// Names the hoisted headers of the function being emitted (`__vh_1`, `__vh_2`, …).
    pub hoist_counter: u32,
    /// `LOFT_HOIST_VERIFY=1` — emit the CHECKING form of every hoisted read, which
    /// re-derives the header and panics when the loop moved the vector under it. Costs
    /// exactly the loads the hoist removed, so it is a gate to run the suite under, not a
    /// production default. See [`hoist`].
    pub hoist_verify: bool,
    /// `LOFT_NO_VECTOR_HOIST=1` — emit every indexed read the way it was emitted before
    /// loft#885. The before-half of an A/B on one binary, and the first thing to try when
    /// a program answers differently under `--native` than under `--interpret`.
    pub hoist_disabled: bool,
    /// `LOFT_NO_ELEM_FUSE=1` — keep loft#885 stage 1 (the loop-invariant header) but emit
    /// stage 2's scalar element reads unfused, as an address and then a load.
    ///
    /// The middle rung of the A/B, and the reason it exists: with only `LOFT_NO_VECTOR_HOIST`
    /// the two stages can be measured together but never apart, so what stage 2 is worth on
    /// top of the header hoist could only be estimated. It also bisects a native-only wrong
    /// answer in a vector loop one stage further than the all-or-nothing switch does.
    pub elem_fuse_disabled: bool,
    /// O7: number of consecutive format/append ops following the current
    /// `OpClearStackText`/`OpClearText`.  Set by `output_block` before each
    /// op is emitted; consumed (and reset to 0) by `clear_stack_text`.
    pub next_format_count: usize,
    /// When true, `Value::Yield(expr)` emits `__values.push((expr) as i64);`
    /// instead of `yield expr`.  Used in the eager-collect factory function
    /// for `ForLoopBody` coroutine segments.
    pub yield_collect: bool,
    /// When set alongside `yield_collect`, the eager-collect buffer is
    /// `Vec<String>` (text-yielding generator) and the `Value::Yield`
    /// emission wraps the value in `.to_string()` instead of `as i64`.
    pub yield_collect_text: bool,
    /// @P326 twin for the eager-collect emitter: the generator yields a
    /// DbRef-shaped value (Reference / Vector / struct-Enum), so a yield
    /// inside a ForLoopBody pushes the DbRef as-is — `as i64` there was
    /// the #481 native E0308/E0605.
    pub yield_collect_dbref: bool,
    /// The store type id an eager factory snapshots each pushed handle under
    /// (`coroutine_snapshot`); `None` when the yield is not a handle, or is a
    /// handle kind with no standalone record (a keyed collection), which the
    /// loop-body push then refuses.
    pub yield_collect_snapshot_tp: Option<u16>,
    /// When set alongside `yield_collect`, the generator yields a TUPLE whose every element
    /// is carried by value, and the eager buffer holds those elements' `i64` images FLAT —
    /// one push per slot, `kinds.len()` per yield.  The `next_into` reader pops the same
    /// stride, so producer and consumer agree by construction (both derive the list from
    /// `coroutine_layout::eager_tuple_kinds` over the same `T`).  `None` for every other
    /// channel, including a tuple carrying a store handle, which the eager collector
    /// refuses (loft#1132).
    pub yield_collect_kinds: Option<Vec<crate::coroutine_layout::YieldSlot>>,
    /// When set alongside `yield_collect`, the eager collector cannot carry this yield type
    /// at all: emit the value bound to `_` plus the `compile_error!` this string holds, so
    /// the generated factory still type-checks and exactly one diagnostic survives.
    pub yield_collect_refuse: Option<String>,
    /// CL-9 (loft#836) — emit a `yield` as the SUSPEND of a lazily-lowered loop: capture the
    /// value into `__y` and leave the single-iteration wrapper, so control returns to the
    /// consumer before the next iteration runs.  Holds the channel's wrap
    /// (`("(", ") as i64")`, `("(", ").to_string()")`, …), the same pair the straight-line
    /// yield arm returns through, so both ends of one generator agree.
    pub yield_lazy_wrap: Option<(String, String)>,
    /// P224: when emitting a coroutine state-machine method body
    /// (`emit_next_i64` / `emit_next_text`), each `var_nr` keyed here
    /// is a function-local that lives as a struct field on the
    /// generator (so its value persists across `next_*` calls).
    /// `Value::Var(v)` for these emits `self.var_<field>` rather than
    /// `var_<name>`; `Value::Set(v, _)` emits `self.var_<field> = …;`.
    /// Empty outside coroutine bodies.
    ///
    /// The VALUE is that field's name, which is usually the variable's own but not always:
    /// two `for i in …` loops in one generator declare two variables spelling `i` and two
    /// spelling `i#index`, and a struct cannot declare a field twice (loft#928).
    /// `coroutine::persistent_field_names` builds the map and is the only place a field name
    /// is derived — keeping membership and spelling in ONE lookup is what stops an emitter
    /// from reading a field under a name the struct definition never used.
    pub coroutine_persistent_fields: HashMap<u16, String>,
    /// Coroutine-persistent vars whose allocating initialiser has already been emitted inside
    /// the current `impl LoftCoroutine`.  A second `Set(v, Null)` on the same field is the
    /// @P302 in-place clear, which must NOT re-run `null_named` (that would orphan the store).
    pub coroutine_allocated_vars: HashSet<u16>,
    /// When true, `Value::Int` emits a `(d_nr_u32, null_DbRef)` tuple
    /// instead of `d_nr_i32`.  Set during fn-ref variable assignment so
    /// if-else branches produce the correct tuple type.
    pub fn_ref_context: bool,
    /// When true, `Value::Int` emits `{v}_i32` instead of the post-2c
    /// default `{v}_i64`.  Set when emitting a tp-number / field-index /
    /// flag-enum slot (where the runtime signature is still i32) so
    /// compile-time constants land at the expected width.  Cleared on
    /// entry to every recursive `output_code_inner` that isn't
    /// explicitly inside such a slot.
    pub i32_literal_context: bool,
    /// When true, `Value::Text` literals inside a `Value::Tuple` emit
    /// with a trailing `.to_string()` so the element fits a
    /// `String`-typed tuple slot.  Set by `set_var` (and similar
    /// destination-aware paths) when assigning a `Value::Tuple` to a
    /// `Type::Tuple(...)` variable that has at least one `Type::Text`
    /// element.  Cleared after the assignment so argument-context
    /// tuples (which need `&str`) keep the default emit.
    pub tuple_text_to_string: bool,
    /// The tuple local an `if` arm is about to hand over, when handing it over would MOVE
    /// it (loft#1354).  Read by the `ValueType::Var` arm, which clones exactly this
    /// variable and nothing else — a blanket clone on every tuple read would copy a
    /// `String` at each use, and most of them are the last one.
    pub(crate) clone_handed_tuple_local: Option<u16>,
    /// loft#1069 — the DECLARED element types of the tuple currently being emitted, when
    /// the destination named them.  Set beside [`Self::tuple_text_to_string`] by the same
    /// destination-aware paths, and for the same reason: a tuple ELEMENT cannot say what
    /// it is.  A fn-ref written as a bare name lowers to the d_nr alone, which infers as
    /// an `integer`, while the slot is the whole `(u32, DbRef)` pair — so only the
    /// destination can ask for the pair to be built.  Empty when the destination is
    /// unknown, and TAKEN by the emitter so a nested tuple does not read its parent's
    /// positions.
    pub tuple_slot_types: Vec<Type>,
    /// When set, `output_block` inserts this code right after the opening `{`.
    /// Used to inject `cr_call_push` / `CallGuard` for shadow call stack support.
    pub call_stack_prefix: Option<String>,
    /// When true, emit `#[no_mangle] pub extern "C" fn loft_start()`
    /// instead of `fn main()` and use WASM imports for native package functions.
    pub wasm_browser: bool,
    /// `--native-wasm`: the emitted Rust is compiled for `wasm32-wasip2`.
    ///
    /// Set beside `wasm_browser` rather than derived from it — they are two
    /// different targets, not two names for one — and the only thing that reads
    /// their disjunction is [`Self::no_c_abi`].
    pub wasm_wasi: bool,
    /// When true, the host-native backend: emit `unsafe extern "C"` declarations
    /// for `#native` package functions (instead of `extern crate <pkg>`) and link
    /// each package's cdylib `.so` by C-ABI, sealing the package's Rust crate graph
    /// inside the `.so` — this eliminates the shared-dep `StableCrateId` collision
    /// class (see NATIVE.md § Resolution: separate the API id from the Rust part).
    /// False for wasm32-wasip2 (links the cross-compiled rlib) and `wasm_browser`
    /// (host imports).
    pub native_cabi: bool,
    /// @PLN26 phase 1 — `#native` symbols exported by 2+ packages.  A *reachable*
    /// call to one is rejected at native codegen (the flat C-ABI namespace can't
    /// disambiguate); two packages sharing an UNUSED symbol still build.  Computed
    /// once from `Data::native_symbol_collisions`.
    pub native_collisions: HashSet<String>,
    /// @PLN18 08-S2 — the live-dispatch fn table, in emission order.  Each
    /// generated user fn with a dispatchable signature opens with
    /// `live_flipped(idx)`; `idx` is its position here.  `emit_native_main`
    /// emits the table as `LOFT_LIVE_FNS` so the runtime bootstrap can
    /// resolve every index against its own parse of the same sources.
    pub live_fns: Vec<String>,
    /// @PLN98 P2 — emit the live/debug tier?  Default `true` keeps the shipped
    /// behaviour (per-fn `live_flipped` entry checks, the `LOFT_LIVE_FNS` table,
    /// and `boot_stores`/`live_enabled` gating in `main`).  The `--lean` opt-out
    /// sets it `false`, so the generated Rust carries ZERO live-dispatch
    /// machinery — the smallest release binary, no live-flip / breakpoints.
    pub emit_live: bool,
    /// @PLN98 P3.1 — the program's own source text, emitted as a `static LOFT_SRC`
    /// blob in a live build so the parked interpreter can bootstrap from EMBEDDED
    /// bytes ([`live_dispatch::bootstrap_from_bytes`](crate::live_dispatch::bootstrap_from_bytes))
    /// with no `LOFT_LIVE_SRC` file — the delivery a browser/wasm build needs.
    /// `None` (the check-only / non-native emit) → `LOFT_SRC = None`, so the boot
    /// falls back to the filesystem path.
    pub program_src: Option<String>,
    /// @PLN98 P3.4 — the browser client's debug NAME (from `--debug[=name]`), baked
    /// into a live `--html` build's `loft_start` so the client can announce itself
    /// to the server, which then ADDRESSES debug frames to it over the relay.
    /// `None` on a production (default, no-`--debug`) client — no debug tier at all.
    pub debug_name: Option<String>,
    /// loft#954 (`--html --names`) — mark every generated loft function
    /// `#[inline(never)]`, so it survives into the wasm as a function of its own.
    ///
    /// Keeping the wasm `name` section is only half of naming a trap's frames.  The
    /// other half is that there has to BE a frame: measured on a four-function
    /// program with loops and vectors, LLVM inlined every one of them into
    /// `loft_start`, so the section named 616 std/alloc internals and not one loft
    /// function.  A backtrace that resolves to `RawVecInner::finish_grow` answers a
    /// question nobody asked.
    ///
    /// Only loft's own functions are pinned — std, alloc and loft's runtime still
    /// inline freely, so the cost is confined to the code the backtrace is about.
    pub keep_fn_names: bool,
}

/// Use this to convert loft names that contain `#` into valid Rust identifiers.
/// Loft uses `#` as a separator in compiler-generated names (e.g., loop iterators).
fn sanitize(name: &str) -> String {
    name.replace('#', "__")
}

/// Use this to determine whether a type is a narrow integer subtype (u8/u16/i8/i16).
/// Returns `Some("u8")` etc. when a cast from `i32` to that type is needed at return sites.
/// Returns `None` for `i32`, `i64`, and all non-integer types.
#[must_use]
fn narrow_int_cast(tp: &Type) -> Option<&'static str> {
    match tp {
        Type::Integer(s) => native_narrow_int(s),
        _ => boolean_u8_cast(tp),
    }
}

/// The narrow Rust type a value of this integer range fits UNENCODED, or `None` when it needs
/// the full width.
///
/// The ONE home for that question, asked by `rust_type`'s Result arms (which pick the
/// signature) and by [`narrow_int_cast`] (which casts the value at the return site).  They
/// have to agree or the generated code does not compile — and they were two copies of the
/// same four-line predicate, which is how they came to share a defect.
///
/// The test is which VALUES the range holds, not how MANY.  Read off the SPAN
/// (`range() - 1 <= 255`) both answered `u8` for `integer limit(300, 400)` — 101 values,
/// which fit a byte only OFFSET-encoded, which is what the store layer does and a register
/// does not.  A body of `350` came back as `94` on `--native` and `350` on `--interpret`,
/// silently, on every build back to the last release; `limit(70000, 70100)` is the cell that
/// shows it is not a u8-vs-u16 boundary question at all, since a 101-wide range picked `u8`
/// for a five-digit number (loft#1255).
///
/// `formal/layout.md` is why the answer is the values: the packed encoding is a STORAGE fact,
/// so the span belongs to `bytes_for_range` and never to a register.  A value in flight is
/// its plain number, which is what the interpreter's i64 slot already does.
fn native_narrow_int(s: &crate::data::IntegerSpec) -> Option<&'static str> {
    let (min, max) = (i64::from(s.min), i64::from(s.max));
    if min >= 0 && max <= 255 {
        Some("u8")
    } else if min >= 0 && max <= 65535 {
        Some("u16")
    } else if min >= -128 && max <= 127 {
        Some("i8")
    } else if min >= -32768 && max <= 32767 {
        Some("i16")
    } else {
        None
    }
}

/// Use this for the `bool` → `u8` coercion alone, without the narrow-integer
/// cases of [`narrow_int_cast`].
///
/// @PLN17: a boolean's storage form is `u8` (0/1/255) while a transient
/// expression is `bool`, so this conversion is needed in EVERY non-expression
/// position — return, store, arg, and the tail of any value-block.  The wrap
/// `(expr) as u8` is idempotent for `u8` and yields 0/1 for `bool`.
/// @PLN25: a nullable boolean (`boolean?`) stores as the same tri-state `u8` —
/// unlike a nullable narrow INTEGER, which is carried full-width `i64`.
#[must_use]
pub(crate) fn boolean_u8_cast(tp: &Type) -> Option<&'static str> {
    match tp {
        Type::Boolean => Some("u8"),
        Type::Optional(inner) if matches!(inner.base(), Type::Boolean) => Some("u8"),
        _ => None,
    }
}

/// The declared type of tuple element `idx` of local `var` — the ONE derivation the
/// tuple-element sites share, so a read, a write and the `set_var` clone rule cannot
/// disagree about what a slot holds.
///
/// Peels the VARIABLE's own marker, so both tuple spellings answer: a by-value
/// `Type::Tuple` local and a `&(…)` reference-tuple parameter (`RefVar(Tuple)`).  The
/// ELEMENT keeps its own marker — a caller asking "is this text?" peels with `.base()`.
///
/// loft#1038 — three sites re-derived this by hand and matched `Type::Text(_)` unpeeled,
/// so a `text?` element was not "text" at any of them.  The read then emitted the
/// `String` BY VALUE where a `text` element emits a borrow, which MOVED the element:
/// `f.0 == null` followed by any later read of `f.0` was rustc E0382 and the program did
/// not build, while `--interpret` answered it correctly.
#[must_use]
pub(crate) fn tuple_elem_type(
    vars: &crate::variables::Function,
    var: u16,
    idx: u32,
) -> Option<Type> {
    let elems = match vars.tp(var).base() {
        Type::Tuple(elems) => elems.clone(),
        Type::RefVar(inner) => match inner.base() {
            Type::Tuple(elems) => elems.clone(),
            _ => return None,
        },
        _ => return None,
    };
    elems.get(idx as usize).cloned()
}

/// Render the base a tuple element access hangs off, and say whether reaching it is
/// `unsafe` — `("var_x", false)` or `("(*var_x)", true)`.
///
/// Reach for this in every position that emits `<base>.<idx>` for a tuple, so the READ
/// and the WRITE cannot disagree about whether the base needs a dereference.  Both facts
/// come from one call because they are one decision: a base that needs the deref needs
/// the `unsafe` too, and answering them separately is how the two arms would drift.
///
/// Three spellings arrive here and only one derefs: a by-value local and a `&(…)`
/// PARAMETER are `(i64, i64)` and `&mut (i64, i64)`, which Rust auto-derefs, while a
/// `&`-bound LOCAL (`b = &a`, `b: &(integer, integer) = a`) holds the raw `*mut (i64, i64)`
/// @PLN87 L1 gives every local link — raw so the source stays readable beside the link,
/// which is legal loft and not legal Rust borrowing.
#[must_use]
pub(crate) fn tuple_base(vars: &crate::variables::Function, var: u16) -> (String, bool) {
    let name = sanitize(vars.name(var));
    if is_raw_tuple_link(vars, var) {
        (format!("(*var_{name})"), true)
    } else {
        (format!("var_{name}"), false)
    }
}

/// True when `var` is a `&`-bound tuple LOCAL, which native represents as a raw
/// `*mut (…)` rather than the `&mut (…)` a `&(…)` PARAMETER gets.
///
/// Every position that hands such a variable to Rust asks this — the element base
/// ([`tuple_base`]) and the call site that forwards it to a `&(…)` parameter — so the
/// representation is decided once.  Two sites reading one predicate is the shape
/// loft#1006 wanted and did not have: there, three copies of the admitted-element list
/// disagreed and a program died at codegen because of it.
#[must_use]
pub(crate) fn is_raw_tuple_link(vars: &crate::variables::Function, var: u16) -> bool {
    !vars.is_argument(var)
        && matches!(vars.tp(var), Type::RefVar(inner) if matches!(inner.base(), Type::Tuple(_)))
}

/// True when tuple element `idx` of `var` is TEXT — nullable or not.  The predicate
/// behind [`tuple_elem_type`]'s most-asked question; see it for why the two are one.
#[must_use]
pub(crate) fn tuple_elem_is_text(vars: &crate::variables::Function, var: u16, idx: u32) -> bool {
    tuple_elem_type(vars, var, idx).is_some_and(|e| matches!(e.base(), Type::Text(_)))
}

/// Use this for the coercion a value-block's TAIL expression needs.
///
/// Integers follow one expression ABI: `i64` everywhere except a function's
/// return signature, which is the only context where [`rust_type`] picks
/// `u8`/`u16`/`i8`/`i16` (`Context::Result`).  A narrow-int cast therefore
/// belongs on a function BODY block and nowhere else.  Applying it to a nested
/// value-block — the `??` null-coalesce `#ncc` block — hands a `u8` to
/// consumers that all expect `i64`, and every one of them then needs its own
/// widen: loft#622's `((v[i] ?? 0) & 255) as u8` failed rustc E0308 because the
/// operand seam of `op_logical_and_int` had none.
///
/// Booleans are the genuine exception and keep casting at every tail — see
/// [`boolean_u8_cast`].
#[must_use]
fn block_tail_cast(tp: &Type, is_fn_body: bool) -> Option<&'static str> {
    if is_fn_body {
        narrow_int_cast(tp)
    } else {
        boolean_u8_cast(tp)
    }
}

/// #433 — does a value placed DIRECTLY into a plain `integer` (i64) destination
/// need an `as i64` widen?  A narrow-int value-block (e.g. a `vec<u8>[i] ?? <int>`
/// null-coalesce) self-casts its tail to the element width (`as u8`) so the
/// element-store / append consumers, which require a narrow input, accept it.  When
/// such a block is instead the RHS of an `integer` return or assignment, that narrow
/// Rust type must be widened back, or rustc rejects with E0308 (`as u8` where i64 is
/// expected).  Other consumers (call args, arithmetic, struct-field / element stores)
/// coerce on their own, so only the two direct-placement seams (Return, Set) consult
/// this.  True when `dest` is a wide integer and `val` is a narrow-integer block.
#[must_use]
fn block_needs_i64_widen(val: &Value, dest: &Type) -> bool {
    matches!(dest, Type::Integer(_))
        && narrow_int_cast(dest).is_none()
        && matches!(val.unspan(), Value::Block(b)
            if matches!(b.result, Type::Integer(_)) && narrow_int_cast(&b.result).is_some())
}

/// @PLN10 — text-returning natives whose generated **wrapper** body returns an
/// owned `String` (their `codegen_runtime` impl was converted off the
/// never-cleared `stores.scratch`).  The wrapper return type (the `-> …` at the
/// function header below) keys on this so wrapper and body never disagree — a
/// blanket "all text wrappers return `String`" flip breaks `#rust` natives
/// (whose bodies stay `Str`-wrapped), `as_text` (null), and cdylib `#native`
/// fns.  Curate this in **lockstep** with the body conversions in
/// `codegen_runtime.rs`.  Sibling of `state::codegen::is_text_dest_native`.
fn native_returns_owned_string(name: &str) -> bool {
    matches!(
        name,
        "i_parse_errors" | "n_json_errors" | "n_parallel_buf_get_text"
    )
}

/// @PLN10 Phase A — a **USER** text-returning function with **no `RefVar(Text)`
/// work buffer** ("nwb") returns an owned `String` rather than a buffer-backed
/// `Str`.  Its wrapper signature is `-> String` and every text return emits
/// `(val).to_string()` (the @PLN10 Phase B owned-String flip).  Two roots map
/// here, both bufferless: `no_work_buffer` (@P205 generic monomorphs, excluded
/// from `text_return` in `definitions.rs`) and any user text fn whose returns are
/// all literal / computed / inner-call (no promoted-local buffer).  Adding **no**
/// buffer means **no** two-pass signature change → it sidesteps Direction-B's
/// instability.  Disjoint from `native_returns_owned_string` (that gates `#native`
/// / `codegen_runtime` stubs; this gates `Block` bodies).
pub(crate) fn def_returns_owned_text(def: &crate::data::Definition) -> bool {
    // nwb = a real loft **Block** body that returns text WITHOUT a `RefVar(Text)`
    // work buffer.  The discriminator is the Block body itself, NOT
    // `rust()/native()` emptiness: an auto-native library fn inlined into a
    // consumer build (main's C71) has a Block body AND `native()` set, yet is
    // still an ordinary user text fn whose buffered-vs-bufferless shape is
    // decided solely by the presence of the work-buffer attribute.  emit.rs only
    // ever runs this on Block bodies, so a non-Block (FFI-direct / `#rust` / Null)
    // body is correctly excluded here and handled by the other owned-String signals.
    // @PLN25 slice (b): peel `Optional` — a `text?` return uses the same owned-text ABI.
    matches!(def.returned().base(), Type::Text(_))
        && matches!(def.code(), Value::Block(_))
        && !def.name().starts_with("Op")
        // A user `&text` parameter is not a work buffer: the buffered-vs-bufferless
        // shape reads the HIDDEN buffers only (`text_work_buffers`), else a function
        // with such a parameter is emitted buffered and returns THROUGH it (loft#1338).
        && def.text_work_buffers() == 0
}

/// @PLN10 — does this function's generated `--native` wrapper return an owned
/// `String` (rather than a buffer-backed `Str`)?  The single source of truth for
/// both the wrapper signature (`output_function`) and any caller that must adapt
/// to it (the shared-store bridge in `native_lib::bridge_write_ret`).  Three
/// disjoint owned-`String` producers:
/// - the curated `codegen_runtime` set (`native_returns_owned_string`);
/// - an FFI-direct text native — `output_native_direct_call` returns the copied
///   `LoftStr` bytes as an owned `String` (N2).  This path is taken ONLY when the
///   body is `Null` (no inlined `Block`), so gate on `code() == Null` to stay
///   aligned with the body selector — an inlined native-lib fn (`Block` body AND
///   `native()` set, main's C71 consumer build) returns `Str`, not `String`;
/// - a bufferless ("nwb") user text fn (`def_returns_owned_text`).
/// - @PLN24 arc D — a **`#c` binding** returning a C `char *`.  Its wrapper owns
///   the copied bytes (`output_c_direct_call` builds the `String`), exactly as an
///   FFI-direct text native does; a `#c` def carries no `native` symbol, so it
///   needs naming here rather than riding the clause above.
pub(crate) fn returns_owned_string(def: &crate::data::Definition) -> bool {
    // @PLN25 slice (b): peel `Optional` — a `text?` return uses the same owned-String ABI.
    matches!(def.returned().base(), Type::Text(_))
        && (native_returns_owned_string(def.name())
            || (*def.code() == Value::Null && !def.native().is_empty())
            || (*def.code() == Value::Null && !def.c_sig.is_empty())
            || def_returns_owned_text(def))
}

/// @PLN130 — the container VARIABLE an element/field read ultimately reads out of.
///
/// One line, because the derivation is [`crate::use_analysis::projection_container_var`]'s,
/// shared with the walk that NAMES the views to materialise (`scopes::base_container_var`).
#[must_use]
pub fn container_element_base(data: &Data, value: &Value) -> Option<u16> {
    crate::use_analysis::projection_container_var(data, value)
}

/// Use this to map a loft type to the Rust type used in generated code.
/// The context controls whether the type appears as an owned value, argument, variable, or reference.
///
/// # Panics
/// When the rust type cannot be determined.
#[must_use]
pub fn rust_type(tp: &Type, context: &Context) -> String {
    if context == &Context::Reference {
        let mut result = String::new();
        result += "&";
        result += &rust_type(tp, &Context::Argument);
        return result;
    }
    if let Type::RefVar(in_tp) = tp {
        return format!("&mut {}", rust_type(in_tp, &Context::Variable));
    }
    match tp {
        // Narrow integer subtypes use their precise Rust type only in the function-return
        // context.  In variable and argument contexts `i32` is used instead to avoid
        // cascading type-mismatch errors when the variable is passed to a template
        // operation (e.g. `set_short`) that expects `i32`.  The `return` site adds an
        // explicit `as u16` / `as u8` cast (see `narrow_int_cast`).
        //
        // WHICH narrow type is `native_narrow_int`'s answer, shared with `narrow_int_cast` so
        // the signature and the cast at the return site cannot disagree (loft#1255).
        Type::Integer(s)
            if context == &Context::Result && native_narrow_int(s).is_some() =>
        {
            native_narrow_int(s).unwrap_or("i64")
        }
        Type::Enum(_, false, _) => "u8",
        Type::Character | Type::Null => "i32",
        Type::Integer(_) => "i64",
        // null is represented as the null sentinel of the target type
        Type::Text(_) if context == &Context::Variable => "String",
        Type::Text(_) if context == &Context::Argument => "&str",
        Type::Text(_) => "Str",
        // @PLN17: boolean tri-state.  Null-capable positions (local/field storage,
        // params, returns) hold the raw byte `u8` (0/1/255); transient expression
        // results are 2-state `bool`.  Mirrors the narrow-int Result→u8 split above
        // and the text String/Str split; coercion to/from `bool` happens at the
        // op-operand (calls.rs `as u8` wrap), store, return, and arg seams.
        Type::Boolean
            if matches!(
                context,
                Context::Variable | Context::Argument | Context::Result | Context::Reference
            ) =>
        {
            "u8"
        }
        Type::Boolean => "bool",
        Type::Float => "f64",
        Type::Single => "f32",
        Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Sorted(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _) | Type::Trie(_, _, _)
        | Type::Enum(_, true, _)
        | Type::Index(_, _, _)
        // N8b.1: generator variables are stored as DbRef (index into native coroutine table).
        | Type::Iterator(_, _) => "DbRef",
        Type::Routine(_) => "u32",
        // C39/A5.6: fn-ref carries d_nr + closure DbRef as a tuple.
        Type::Function(_, _, _) => "(u32, DbRef)",
        Type::Unknown(_) => "??",
        Type::Keys => "&[Key]",
        Type::Void => "()",
        // N8a.1: emit the correct Rust tuple type, e.g. (i32, i64) for (integer, long).
        // T1.8a: in Result context, recurse with Variable context for element
        // types so a tuple-of-text return signature is `(String, String)` —
        // matching the caller's owned-tuple slot.  Without this the signature
        // becomes `(Str, Str)` (Result-context Text → "Str") while the
        // caller's local declares `(String, String)` (Variable-context),
        // producing a type mismatch.  Argument/Variable contexts already
        // recurse with their own context (giving `(&str, &str)` or
        // `(String, String)` respectively), which matches the call site
        // and the variable slot.
        Type::Tuple(elems) => {
            let elem_context = if matches!(context, Context::Result) {
                &Context::Variable
            } else {
                context
            };
            let parts: Vec<String> = elems.iter().map(|e| rust_type(e, elem_context)).collect();
            return format!("({})", parts.join(", "));
        }
        // @PLN25: a nullable narrow integer is carried FULL-WIDTH (i64) as a Rust value.
        // The narrow packing is store-only; a narrow Rust type (u8/u16) can't hold the
        // null sentinel alongside the full value range (`u8?` must keep 255-as-value
        // distinct from null). Non-Result contexts already map a narrow Integer to i64,
        // so this only corrects the Result (function-return) signature to match the i64
        // the body yields — else a `-> u8` header over a `return <i64>` body is rustc E0308.
        Type::Optional(inner) if matches!(inner.base(), Type::Integer(_)) => {
            return "i64".to_string();
        }
        // @PLN25 slice (b): `Optional(τ)` shares its base's Rust type (sentinel storage).
        Type::Rewritten(inner) | Type::Optional(inner) => return rust_type(inner, context),
        _ => panic!("Incorrect type {tp:?}"),
    }
    .to_string()
}

/// Return the Rust literal for the "null" default of a loft type, used when a function
/// body is empty (an explicit stub) but the declared return type is non-void.
/// #354: locals whose first WRITE sits inside a nested block while a later
/// use occurs OUTSIDE that block's subtree.  loft locals are FUNCTION-scoped
/// frame slots (the interpreter's stack), but the emitter writes `let` at
/// the first assignment — Rust then scopes the variable to that block, and
/// a sibling-block use fails E0425 ("variables lost across block-split
/// boundaries": `intown = …` inside one `if depth == 0 { }`, read in a later
/// one).  These variables get a typed default declaration in the function
/// prologue instead.
fn collect_scope_hoists(code: &Value) -> std::collections::HashSet<u16> {
    use std::collections::{HashMap, HashSet};
    fn walk(
        node: &Value,
        path: &mut Vec<u32>,
        next_id: &mut u32,
        first_set: &mut HashMap<u16, Vec<u32>>,
        hoist: &mut HashSet<u16>,
    ) {
        // A use (read or re-write) outside the first write's block subtree
        // means the `let` position cannot cover it.
        fn check_use(
            v: u16,
            path: &[u32],
            first_set: &HashMap<u16, Vec<u32>>,
            hoist: &mut HashSet<u16>,
        ) {
            if let Some(p) = first_set.get(&v)
                && !path.starts_with(p)
            {
                hoist.insert(v);
            }
        }
        // Enter a child node that the emitter wraps in its own `{ … }`.
        macro_rules! scoped {
            ($child:expr) => {{
                *next_id += 1;
                path.push(*next_id);
                walk($child, path, next_id, first_set, hoist);
                path.pop();
            }};
        }
        match node {
            Value::Set(v, rhs) => {
                walk(rhs, path, next_id, first_set, hoist);
                if first_set.contains_key(v) {
                    check_use(*v, path, first_set, hoist);
                } else {
                    first_set.insert(*v, path.clone());
                }
            }
            Value::TuplePut(v, _, rhs) => {
                walk(rhs, path, next_id, first_set, hoist);
                check_use(*v, path, first_set, hoist);
            }
            Value::Var(v) => check_use(*v, path, first_set, hoist),
            Value::Block(bl) | Value::Loop(bl) => {
                *next_id += 1;
                path.push(*next_id);
                for op in &bl.operators {
                    walk(op, path, next_id, first_set, hoist);
                }
                path.pop();
            }
            Value::If(c, t, f) => {
                walk(c, path, next_id, first_set, hoist);
                scoped!(t);
                scoped!(f);
            }
            Value::Iter(_, a, b, c) => {
                scoped!(a);
                scoped!(b);
                scoped!(c);
            }
            Value::Insert(ops) => {
                *next_id += 1;
                path.push(*next_id);
                for op in ops {
                    walk(op, path, next_id, first_set, hoist);
                }
                path.pop();
            }
            Value::Call(_, args)
            | Value::CallRef(_, args)
            | Value::Tuple(args)
            | Value::Parallel(args) => {
                for a in args {
                    walk(a, path, next_id, first_set, hoist);
                }
            }
            Value::Return(x) | Value::Drop(x) | Value::Yield(x) => {
                walk(x, path, next_id, first_set, hoist);
            }
            Value::Span(b) => walk(&b.1, path, next_id, first_set, hoist),
            _ => {}
        }
    }
    let mut first_set = HashMap::new();
    let mut hoist = HashSet::new();
    let mut path = Vec::new();
    let mut next_id = 0u32;
    walk(code, &mut path, &mut next_id, &mut first_set, &mut hoist);
    hoist
}

/// @FR-E-Uncomp-NN — the value a slot of `tp` holds when nobody assigned one.
///
/// NOT [`default_native_value`], which despite its name answers the type's NULL: a
/// `boolean`'s `255u8` sentinel, a text's `STRING_NULL`, a flat `0` for every integer
/// whatever its declared range.  That is the right answer where a null is what is wanted —
/// an explicit `return null`, a `Value::Null` store — and the wrong one for a NON-NULLABLE
/// destination, which is what an empty-bodied function's return is: `fn f() -> boolean { }`
/// answered `null` natively where the interpreter said `false`, and
/// `fn f() -> integer limit(10, 20) { }` answered `0`, a value that type does not have
/// (loft#1254).
///
/// Only the two arms that differ are stated here; everything else delegates, because for
/// every other type the null IS the zero-shaped default (an empty collection's `DbRef::NULL`,
/// `0.0` for a float).  `character` is the documented exception and stays: its null is
/// codepoint 0 and so is its default, so the collision `formal/types.md` records for `Char`
/// leaves nothing better to return.
pub(super) fn uninitialised_native_value(tp: &Type, context: &Context) -> String {
    match tp {
        // The storage form of `false`; the null is `255u8`.
        Type::Boolean => "0u8".into(),
        Type::Integer(spec) => spec.default_value().to_string(),
        // `to_default` says a text's default is the EMPTY string (@FR-D-Text); the null is
        // the `STRING_NULL` sentinel `default_native_value_in` answers, which is a different
        // value and the one an uninitialised slot must not take.  Spelled per context for
        // the reason that function's own doc gives: `String` where the destination is
        // declared as one, `Str` in a return.
        Type::Text(_) if context == &Context::Variable => "String::new()".into(),
        Type::Text(_) => "Str::new(\"\")".into(),
        Type::Optional(inner) => default_native_value_in(inner, context),
        _ => default_native_value_in(tp, context),
    }
}

/// The native-Rust DEFAULT-INIT literal for a variable / return of type `tp` —
/// the placeholder a `let mut var_x: T = …;` gets before its real assignment
/// (and the value emitted for an explicit `var = null`, `dispatch.rs`).
///
/// This is the default-INIT contract, DISTINCT from the live NULL sentinel that
/// `emit_typed_null` (`state/codegen.rs`) pushes on the bytecode stack
/// (H4 — keep the two in step; do NOT assume they're interchangeable):
/// - For bool / text / reference / collection types the default-init IS the
///   null storage form (`255u8` / `STRING_NULL` / `DbRef::NULL`), so it
///   coincides with the null.
/// - For SCALARS (`Integer`, `Character`, `Float`, `Single`) it is the ZERO
///   default (`0` / `0.0`), NOT the null sentinel (`i64::MIN` / `NaN`).  Safe
///   because a live scalar null never flows through here: the live-null path
///   emits the sentinel on BOTH backends (verified — a `null`-returning
///   `integer`/`float` fn reads back as the sentinel on interp AND native), and
///   `floatvar = null` is type-rejected.  Do NOT route a live scalar null
///   through this — it would diverge from `emit_typed_null`.
pub(super) fn default_native_value(tp: &Type) -> String {
    default_native_value_in(tp, &Context::Result)
}

/// [`default_native_value`] in the context the destination is DECLARED in.
///
/// Only `text` spells its null differently per context — `Str` in a return, `String` in a
/// local — and a declaration site that paired `rust_type(tp, &Context::Variable)` with the
/// context-free literal emitted `let mut var_x: String = Str::new(…)`, which is rustc
/// E0308.  The tuple arm below had already patched exactly this by hand for its elements;
/// this is the same fact, asked once (loft#1016).
pub(super) fn default_native_value_in(tp: &Type, context: &Context) -> String {
    match tp {
        Type::Float => "0.0_f64".into(),
        Type::Single => "0.0_f32".into(),
        // @PLN17: a boolean's null default is the 255 sentinel (storage form u8).
        Type::Boolean => "255u8".into(),
        // A local / variable slot is a `String` (`rust_type`'s Variable context), a
        // return is a `Str` — the null CONTENT is the same sentinel either way.
        Type::Text(_) if context == &Context::Variable => {
            "loft::state::STRING_NULL.to_string()".into()
        }
        Type::Text(_) => "Str::new(loft::state::STRING_NULL)".into(),
        // @PLN25 slice (c): `Optional(τ)` has `τ`'s native default (a `text?` null is the
        // `Str` sentinel, exactly like `text`) — without this it fell to the `0` catch-all.
        Type::Optional(inner) => default_native_value_in(inner, context),
        Type::Routine(_) => "0_u32".into(),
        Type::Function(_, _, _) => "(0_u32, DbRef::NULL)".into(),
        Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Sorted(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _) | Type::Trie(_, _, _)
        | Type::Enum(_, true, _)
        | Type::Index(_, _, _)
        // N8b.1: exhausted / uninitialized generator variable.
        // The canonical heap-ref null is `DbRef::NULL` (`keys.rs`) — the one
        // source; `is_null()` ignores `pos`, so the old `pos: 8` was drift
        // (Cluster D / H6), now converged.
        | Type::Iterator(_, _) => "DbRef::NULL".into(),
        // N8a.1: a tuple null is the zero-default for each element type.
        Type::Tuple(elems) => {
            // Tuple variables hold Variable-context element types
            // (`String` for `Text`, etc.), so the per-element default
            // must match.  Map `Type::Text` to `String::new()` here
            // — the bare `default_native_value(Text)` would return
            // `Str::new(...)` which is the Argument-context literal
            // and won't fit a `String`-typed tuple slot.
            let parts: Vec<String> = elems
                .iter()
                .map(|e| default_native_value_in(e, &Context::Variable))
                .collect();
            format!("({})", parts.join(", "))
        }
        _ => "0".into(), // Integer, Character, Enum(u8), etc.
    }
}

/// Which subset of a struct / enum-value's attributes to emit in the
/// current pass of `output_init`.  Phase 1 emits `Simple` fields so
/// bare Sorted/Hash/Index types registered later find their content
/// struct already populated.  Phase 2 emits `Collection` fields (which
/// reference those pre-created bare collections via `t{N}`) and
/// `EnumValues` (the `db.value` add-backs that close the enum ↔
/// variant mutual-recursion cycle).
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq)]
enum FieldPhase {
    /// Emit ALL struct / enum-value fields in source order.  Collection
    /// fields (Sorted/Hash/Index/Vector) trigger inline `db.sorted /
    /// hash / index / vector` creation that dedups on name — the
    /// runtime id assigned at first creation matches the compile-time
    /// `known_type`, so subsequent references through raw literals
    /// (`OpNewRecord(parent_tp, field_index)`) stay correct.
    AllFields,
    /// Emit only the `db.value(...)` add-backs for enum values.
    EnumValues,
    /// (Historical — kept for potential partial emission.)
    Simple,
    Collection,
}

/// Return true when the given field type participates in Phase 2
/// (collection-typed fields that reference a bare Vector / Sorted /
/// Hash / Index created during `output_init`'s first pass).
fn is_collection_field(tp: &Type) -> bool {
    // Radix backs `spatial<T[x,y]>` (@PLN48) — same Phase-2 bare-type
    // reference shape as Hash, so it is classified alongside the family.
    // One home: `vectors::is_collection`, which DERIVES this as `is_keyed(tp) || Vector`
    // rather than restating the six variants (formal/IMPLEMENTATIONS.md #4).
    crate::parser::vectors::is_collection(tp)
}

/// A "bare" runtime type (no loft definition) that `output_init` must
/// re-create so its runtime type id matches its compile-time
/// `known_type`.  Collected from `Stores::types` and emitted in id
/// order, interleaved with the struct/enum definitions.  See
/// `Output::flush_bare_through` for the emission contract.
#[allow(dead_code)]
enum BareIo {
    Byte(i32, bool),
    Short(i32, bool),
    ShortRaw(i32, bool),
    Int(i32, bool),
    Vector(u16),
    Sorted(u16, Vec<(u16, bool)>),
    Hash(u16, Vec<u16>),
    Radix(u16, Vec<u16>),
    Trie(u16, u16),
    Index(u16, Vec<(u16, bool)>),
}

impl<'a> Output<'a> {
    /// All-defaults constructor — the ONE place that knows the field list.
    /// Eleven hand-rolled struct literals (src + tests) previously each
    /// enumerated every field and drifted one field at a time on every
    /// addition (pass-3 dedupe).  Callers flip the rare non-default flag
    /// (`wasm_browser`) on the returned value.
    #[must_use]
    pub fn new(data: &'a Data, stores: &'a Stores) -> Self {
        Output {
            data,
            stores,
            counter: 0,
            def_nr: 0,
            indent: 0,
            declared: HashSet::new(),
            local_record_link: HashSet::new(),
            retbuf_witness: HashSet::new(),
            witness_vars: HashSet::new(),
            predeclared: HashSet::new(),
            active_pre_eval: HashMap::new(),
            reachable: HashSet::new(),
            dup_fn_names: HashSet::new(),
            loop_stack: Vec::new(),
            vec_headers: Vec::new(),
            hoist_cache: HashMap::new(),
            hoist_counter: 0,
            hoist_verify: std::env::var("LOFT_HOIST_VERIFY").is_ok_and(|v| v != "0"),
            hoist_disabled: std::env::var("LOFT_NO_VECTOR_HOIST").is_ok_and(|v| v != "0"),
            elem_fuse_disabled: std::env::var("LOFT_NO_ELEM_FUSE").is_ok_and(|v| v != "0"),
            next_format_count: 0,
            yield_collect: false,
            yield_collect_text: false,
            yield_collect_dbref: false,
            yield_collect_snapshot_tp: None,
            yield_collect_kinds: None,
            yield_collect_refuse: None,
            yield_lazy_wrap: None,
            coroutine_persistent_fields: HashMap::new(),
            coroutine_allocated_vars: HashSet::new(),
            fn_ref_context: false,
            i32_literal_context: false,
            tuple_text_to_string: false,
            clone_handed_tuple_local: None,
            tuple_slot_types: Vec::new(),
            call_stack_prefix: None,
            wasm_browser: false,
            wasm_wasi: false,
            native_cabi: false,
            native_collisions: data
                .native_symbol_collisions()
                .into_iter()
                .map(|(s, _)| s)
                .collect(),
            live_fns: Vec::new(),
            emit_live: true,
            program_src: None,
            debug_name: None,
            keep_fn_names: false,
        }
    }

    /// loft#954 — the attribute line that precedes a generated loft function
    /// definition, so `--names` keeps it out of line and a trap's frame can name it.
    /// Empty (not a blank line) in every ordinary build, which keeps the emitted
    /// Rust byte-identical to what it was.
    #[must_use]
    fn fn_inline_attr(&self) -> &'static str {
        if self.keep_fn_names {
            "#[inline(never)]\n"
        } else {
            ""
        }
    }

    /// @PLN24 arc E — is this target one a `#c` binding cannot reach?
    ///
    /// True for both wasm shapes. A wasm module has no `dlopen`, so a declared
    /// `[c] libs` soname is unreachable by construction; and where a symbol IS
    /// satisfiable — `wasm32-wasip2` links a libc, so `strlen` resolves — it is
    /// satisfied under a THIRD data model (ILP32: `long`, `size_t` and every
    /// pointer are 32 bits) that the host-derived `CTarget` does not describe.
    /// Measured, and worth stating because the plan originally recorded wasm as
    /// having "no C ABI at all": that binding LINKED, with a warning, and then
    /// trapped at the call.
    ///
    /// One reader, so the two target flags cannot drift into two different
    /// answers at the three sites that consult them — the `extern` block, the
    /// lazy resolver, and the call itself.
    #[must_use]
    pub fn no_c_abi(&self) -> bool {
        self.wasm_browser || self.wasm_wasi
    }
}

/// @PLN90 #495 — collect the "runtime-Join" locals of function `def_nr` (see
/// [`Output::witness_vars`]).  A var qualifies iff it is an owned-typed
/// Reference / struct-Enum local with ≥1 owned-producing assignment (a whole-value
/// copy, or an owned call / struct literal) AND ≥1 borrow-producing reassignment
/// (an `?? `/ncc block whose value borrows a view or a param).  That owned + borrow
/// mix is precisely the runtime JOIN: r owns a store on the path where an owned
/// assign last ran, and borrows on the path where the ncc last ran — so a static
/// owned-type free would whole-store-free the view.  A SECOND owned assign (an
/// owned reassign) is handled too: `output_set` resets r to the null sentinel
/// before the value so it allocates fresh instead of reusing / freeing the view r
/// currently holds.
fn collect_witness_vars(data: &crate::data::Data, def_nr: u32) -> HashSet<u16> {
    if !crate::keys::join_own_enabled() {
        return HashSet::new();
    }
    // (owned_count, borrow_count) per candidate var.
    let mut counts: HashMap<u16, (u32, u32)> = HashMap::new();
    fn classify_set(
        data: &crate::data::Data,
        def_nr: u32,
        v: u16,
        to: &Value,
        counts: &mut HashMap<u16, (u32, u32)>,
    ) {
        let vars = data.def(def_nr).variables();
        // @FR-O-Proxy asks oracle — this counts owned-vs-borrow assignments to pick the
        // witness vars, and takes the real per-value answer from @FR-O-Oracle below
        // (`ownership_of`).  The proxy only narrows the candidate set; it frees nothing.
        let is_candidate = !vars.is_argument(v)
            && matches!(vars.tp(v), Type::Reference(_, _) | Type::Enum(_, true, _))
            && vars.tp(v).depend().is_empty()
            // loft#1336 / @FR-O-Witness — a local the IR already tracks through its owner
            // witness is left to the IR: two trackers of one store would free it twice.
            && vars.owner_witness(v).is_none();
        if !is_candidate {
            return;
        }
        let owned = match to.unspan() {
            // A whole-value copy of another heap var — native emits `OpCopyRecord`
            // into a fresh store, so r OWNS the result (C86), regardless of the
            // source's own ownership.
            Value::Var(src) if vars.tp(*src).heap_def_nr().is_some() => true,
            // An owned call / struct literal is Owned; an `?? `/ncc block is a
            // Borrow/Join view — the oracle carries the distinction.
            Value::Block(_) | Value::Call(_, _) | Value::Insert(_) => matches!(
                crate::use_analysis::ownership_of(data, def_nr, to),
                crate::use_analysis::Own::Owned
            ),
            // Not a heap-store-producing assign (scalar, null, …).
            _ => return,
        };
        let e = counts.entry(v).or_insert((0, 0));
        if owned {
            e.0 += 1;
        } else {
            e.1 += 1;
        }
    }
    fn walk(
        data: &crate::data::Data,
        def_nr: u32,
        node: &Value,
        counts: &mut HashMap<u16, (u32, u32)>,
    ) {
        match node {
            Value::Set(v, to) => {
                classify_set(data, def_nr, *v, to, counts);
                walk(data, def_nr, to, counts);
            }
            Value::Span(b) => walk(data, def_nr, &b.1, counts),
            Value::Block(b) | Value::Loop(b) => {
                for op in &b.operators {
                    walk(data, def_nr, op, counts);
                }
            }
            Value::Insert(ops) | Value::Parallel(ops) => {
                for op in ops {
                    walk(data, def_nr, op, counts);
                }
            }
            Value::If(c, t, e) => {
                walk(data, def_nr, c, counts);
                walk(data, def_nr, t, counts);
                walk(data, def_nr, e, counts);
            }
            Value::Iter(_, a, b2, c) => {
                walk(data, def_nr, a, counts);
                walk(data, def_nr, b2, counts);
                walk(data, def_nr, c, counts);
            }
            Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
                walk(data, def_nr, v, counts);
            }
            Value::Call(_, args) => {
                for a in args {
                    walk(data, def_nr, a, counts);
                }
            }
            _ => {}
        }
    }
    walk(data, def_nr, data.def(def_nr).code(), &mut counts);
    counts
        .into_iter()
        .filter(|(_, (owned, borrow))| *owned >= 1 && *borrow >= 1)
        .map(|(v, _)| v)
        .collect()
}

impl Output<'_> {
    /// The Rust identifier this generation emits for function `def` — the bare
    /// name unless it collides across modules (see [`disambiguated_fn_ident`]).
    /// Every site that writes a fn definition OR a call to one must go through
    /// this (#305).
    #[must_use]
    pub fn fn_ident(&self, def: &crate::data::Definition) -> String {
        disambiguated_fn_ident(&self.dup_fn_names, def)
    }

    /// The Rust place for loft variable `var` — the generator's struct field when the
    /// function being emitted is a coroutine and this local persists across resumes, and a
    /// plain `var_<name>` local otherwise.
    ///
    /// Every site that names a variable as somewhere to READ FROM or WRITE TO has to ask,
    /// because a persistent local has no `let` of its own: the factory made it a field, so
    /// spelling it as a local emits a Rust identifier that does not exist.  Appending to a
    /// text local inside a generator did exactly that — `a = a + t` in a loop emitted
    /// `var_a += …` against a field named `self.var_a`, and `--native` refused to compile
    /// a program `--interpret` ran correctly.
    ///
    /// Outside a coroutine the map is empty and this is the bare local every site spelled
    /// before, so routing a site through it can only add the case it was missing.
    #[must_use]
    pub fn var_place(&self, var: u16) -> String {
        match self.coroutine_persistent_fields.get(&var) {
            Some(field) => format!("self.var_{field}"),
            None => format!(
                "var_{}",
                sanitize(self.data.def(self.def_nr).variables().name(var))
            ),
        }
    }

    /// Use this before emitting indented output lines.
    /// # Errors
    /// When the output cannot be written
    pub fn indent(&self, w: &mut dyn Write) -> std::io::Result<()> {
        for _i in 0..=self.indent {
            write!(w, "  ")?;
        }
        Ok(())
    }

    /// Use this to reset the emission state when starting a new function.
    pub fn start_fn(&mut self, def_nr: u32) {
        self.def_nr = def_nr;
        self.indent = 0;
        self.declared.clear();
        self.local_record_link.clear();
        self.retbuf_witness.clear();
        self.witness_vars.clear();
        self.predeclared.clear();
        self.next_format_count = 0;
        self.vec_headers.clear();
        self.hoist_counter = 0;
    }

    /// loft#885 — open a loop with the headers of the vectors it only reads.
    ///
    /// Emits `{ let __vh_N = vector::vec_header(…);` per hoistable vector and answers
    /// whether that wrapper block was opened, which [`Self::end_vector_hoist`] closes. The
    /// wrapper is what makes the prelude legal wherever a loop can appear — a loop is an
    /// expression in Rust, and a bare `let` before one would not be.
    ///
    /// A vector already covered by an enclosing loop's prelude is skipped: the enclosing
    /// header is still current, because the promise that let it be hoisted covers this
    /// loop too.
    fn begin_vector_hoist(
        &mut self,
        w: &mut dyn Write,
        lp: &crate::data::Block,
    ) -> std::io::Result<bool> {
        let mut frame: HashMap<u16, String> = HashMap::new();
        let candidates = if self.hoist_disabled {
            Vec::new()
        } else {
            hoist::hoistable_vectors(lp, self.data, self.def_nr, &mut self.hoist_cache)
        };
        let mut lines: Vec<String> = Vec::new();
        for v in candidates {
            if self.vec_headers.iter().any(|f| f.contains_key(&v)) {
                continue;
            }
            // A generator's locals live on the generator struct and are named `self.var_x`
            // inside its `next` methods. The prelude below spells a plain local, so a
            // vector that moved onto the struct is left alone rather than named wrongly.
            if self.coroutine_persistent_fields.contains_key(&v) {
                continue;
            }
            self.hoist_counter += 1;
            let name = format!("__vh_{}", self.hoist_counter);
            let var = sanitize(self.data.def(self.def_nr).variables().name(v));
            lines.push(format!(
                "let {name} = vector::vec_header(&(var_{var}), &stores.allocations);"
            ));
            frame.insert(v, name);
        }
        let opened = !lines.is_empty();
        if opened {
            writeln!(w, "{{ //loft#885 loop-invariant vector headers")?;
            for line in lines {
                self.indent(w)?;
                writeln!(w, "{line}")?;
            }
            self.indent(w)?;
        }
        self.vec_headers.push(frame);
        Ok(opened)
    }

    /// Close what [`Self::begin_vector_hoist`] opened.
    fn end_vector_hoist(&mut self, w: &mut dyn Write, opened: bool) -> std::io::Result<()> {
        self.vec_headers.pop();
        if opened {
            write!(w, " }}")?;
        }
        Ok(())
    }

    /// The Rust local holding `var`'s hoisted header, when an enclosing loop derived one.
    #[must_use]
    pub fn active_vec_header(&self, var: u16) -> Option<&str> {
        self.vec_headers
            .iter()
            .rev()
            .find_map(|f| f.get(&var).map(String::as_str))
    }

    /// Whether this call is emitted as ONE fused element read (loft#885 stage 2) — the
    /// shape qualifies, the enclosing loop hoisted a header for the vector, and the fusion
    /// is not switched off by `LOFT_NO_ELEM_FUSE`.
    ///
    /// All three conditions live here because the emitter and the pre-eval collector have
    /// to reach the SAME verdict: the collector hoists an inner `OpGetVector*` into a
    /// `let _pre_N` that a fused emission ignores, so a disagreement would run the read
    /// twice. One answer, asked twice, cannot disagree with itself.
    #[must_use]
    pub fn fused_element_read<'a>(
        &self,
        getter: &str,
        args: &'a [Value],
    ) -> Option<hoist::FusedRead<'a>> {
        if self.elem_fuse_disabled {
            return None;
        }
        let fused = hoist::fused_element_read(self.data, getter, args)?;
        self.active_vec_header(fused.var)?;
        Some(fused)
    }

    /// @PLN18 08-S2 — build the live-dispatch entry check for a user fn, or
    /// `None` when its signature is outside the v1 dispatchable set (text /
    /// character / fn-ref / `&mut` args, narrow-int / text returns).  The
    /// check is ONE relaxed atomic load when live mode is off; when the fn is
    /// flipped it re-enters the parked interpreter over the shared world,
    /// pushing args in declared order (the 02 frame contract).  Allocates the
    /// fn's table index as a side effect — gate first, allocate after.
    /// Every local a `return` in this body NAMES, at any depth.
    ///
    /// A Rust `let` lives where the emission first reaches it, so a local first assigned
    /// inside a nested block is out of scope for a `return` outside it.  The interpreter
    /// cannot have that — a local is a frame slot wherever it is written — so the answer is
    /// used to bind those locals up front and make the two backends agree about scope.
    fn collect_returned_vars(node: &Value, out: &mut Vec<u16>) {
        if let Value::Return(inner) = node.unspan()
            && let Value::Var(v) = inner.unspan()
            && !out.contains(v)
        {
            out.push(*v);
        }
        node.unspan()
            .for_each_child(&mut |child| Self::collect_returned_vars(child, out));
    }

    fn live_entry_check(&mut self, def: &crate::data::Definition) -> Option<String> {
        if def.name() == "n_main" {
            return None;
        }
        use std::fmt::Write as _;
        let mut pushes = String::new();
        for a in def.attributes() {
            match &a.typedef {
                Type::Integer(_)
                | Type::Float
                | Type::Boolean
                | Type::Reference(_, _)
                | Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _) => {
                    let _ = write!(pushes, " st.put_stack(var_{});", sanitize(&a.name));
                }
                _ => return None,
            }
        }
        let thunk = match def.returned() {
            Type::Void => "live_call_void",
            Type::Integer(_) if rust_type(def.returned(), &Context::Result) == "i64" => {
                "live_call_i64"
            }
            Type::Float => "live_call_f64",
            Type::Boolean => "live_call_u8",
            Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _) => "live_call_ref",
            _ => return None,
        };
        let idx = self.live_fns.len();
        self.live_fns.push(def.name().to_string());
        Some(format!(
            "  if loft::live_dispatch::live_flipped({idx}) {{ return loft::live_dispatch::{thunk}(cell, {idx}, |st| {{{pushes} }}); }}\n"
        ))
    }

    /// Emit the common Rust file header (attributes, imports, `mod external`).
    ///
    /// `reachable` filters the `extern crate <pkg>` declarations to the native
    /// packages the emitted code actually calls (empty = whole-program
    /// fallback, keep all) — see the comment at the emission loop (#307).
    fn emit_file_header(
        w: &mut dyn Write,
        data: &Data,
        wasm_browser: bool,
        no_c_abi: bool,
        native_cabi: bool,
        reachable: &HashSet<u32>,
    ) -> std::io::Result<()> {
        // Every `allow` here suppresses a rustc lint about GENERATED code, which a user
        // cannot act on and did not write: the emitter names every local whether or not
        // the body reads it, parenthesises defensively, and emits whatever the IR says.
        // `unused_must_use` and `path_statements` joined the list for loft#1075: a
        // `Value::Drop` is the IR saying *discard this value*, so a `#[must_use]` runtime
        // op or a bare local reached as a statement (`a + 1;`, `n;`, or a value tail in a
        // void function) is loft doing exactly what it was told — and the warning arrived
        // in the user's terminal quoting a line of a temporary `.rs` file.
        writeln!(
            w,
            "\
#![allow(unused_imports)]
#![allow(unused_parens)]
#![allow(unused_variables)]
#![allow(unreachable_code)]
#![allow(unused_mut)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(redundant_semicolons)]
#![allow(unused_assignments)]
#![allow(unused_labels)]
#![allow(unused_braces)]
#![allow(unused_must_use)]
#![allow(path_statements)]
#![allow(clippy::double_parens)]
#![allow(clippy::unused_unit)]
#![allow(unused_unsafe)]

extern crate loft;"
        )?;
        if wasm_browser {
            // declare host-imported functions for browser WASM.
            writeln!(w, "#[link(wasm_import_module = \"loft_io\")]")?;
            writeln!(w, "unsafe extern \"C\" {{")?;
            writeln!(
                w,
                "    safe fn loft_host_print(ptr: *const u8, len: usize);"
            )?;
            writeln!(w, "}}")?;
            // W1.1 step 6: emit WASM import declarations for all #native functions.
            // Each native symbol gets declared as an imported extern "C" function so
            // the generated code can call it directly (unqualified).
            writeln!(w, "#[link(wasm_import_module = \"loft_gl\")]")?;
            writeln!(w, "unsafe extern \"C\" {{")?;
            let mut declared_natives = std::collections::HashSet::new();
            for d_nr in 0..data.definitions() {
                let def = data.def(d_nr);
                if def.native().is_empty() || declared_natives.contains(def.native()) {
                    continue;
                }
                // #407 — skip the host-import extern declaration for ANY
                // `#native` routed through `[wasm.bridge].routes`.  The local
                // wrapper body is a call into `<crate>::<bridge_fn>` (see
                // `output_wasm_bridge_call`), so the extern would be unused AND
                // would collide with that wrapper's name (E0428) — exactly the
                // blocker for a `text -> text` bridge, whose extern decl
                // (`safe fn n_x(ptr,len) -> i32`) clashed with the loft wrapper
                // `fn n_x(...) -> String`.  The bridge crate declares its OWN
                // host imports, so no extern is needed at this layer.
                if data.wasm_bridge_routes.contains_key(def.native()) {
                    continue;
                }
                // @P321c browser-WASM (2026-05-29): skip the host-import
                // declaration for store-mutating `#native` fns with NO bridge —
                // the matching local body is a graceful Phase 1 stub (see
                // `output_native_direct_call`), so the extern would be unused
                // AND would collide with the local body name (E0428).
                let stores_loft_ref =
                    matches!(def.returned(), Type::Vector(_, _) | Type::Reference(_, _));
                // A heap-typed arg (Reference / Vector / data-enum / sorted /
                // hash / index / spatial) is passed as a `LoftStore` + `LoftRef`
                // handle, exactly as the interpreter's `ArgT::Ref`/`ArgT::Vec`
                // marshal it — NOT a raw `(ptr, count)` pair.  `heap_dep()` is the
                // canonical set of those types (the same union the interpreter
                // groups in `compute_sig`); reuse it so the two backends agree.
                // D-html-vec (2026-07-04): a `vector<T>` arg IS a valid host import —
                // it marshals as `(ptr, count)` (declared just below), which the JS glue
                // reads as `new Float32Array(mem, ptr, count)`.  Only NON-vector heap args
                // (Reference / keyed collections) need a LoftRef handle a host import can't
                // take, so only those skip.  #423 broadened this to EVERY heap_dep and
                // silently dropped gl_upload_vertices / gl_upload_canvas / gl_set_mat4 host
                // imports → Brick Buster (and any --html WebGL program) rendered blank.
                let has_ref_arg = def.attributes().iter().any(|a| {
                    !a.name.starts_with("__")
                        && a.typedef.heap_dep().is_some()
                        && !matches!(a.typedef, Type::Vector(_, _))
                });
                if stores_loft_ref || has_ref_arg {
                    continue;
                }
                // #623 — a REACHABLE routeless `#native` whose SYMBOL equals the
                // loft fn's own emitted name (`n_<fn>`, i.e. a bare `#native`)
                // gets both this host-import declaration and a local wrapper body
                // under that one name — an unreadable rustc cascade (E0428 plus a
                // string of E0610/E0061 against generated code, rustc's "remove
                // the extra argument" pointing nowhere useful).  Skip the extern
                // so exactly one diagnostic survives: the `compile_error!` the
                // body site emits, naming the symbol, its library and the fix.
                //
                // Two shapes deliberately still get their declaration:
                //   * an UNREACHABLE routeless native — unused, emits no body, and
                //     must not reject an otherwise-valid program;
                //   * a native whose symbol DIFFERS from the fn name
                //     (`#native "loft_gl_swap_buffers"` on `gl_swap_buffers`) —
                //     that is the legitimate raw host-import path the browser
                //     shell supplies, and it never collided.
                if reachable.contains(&d_nr)
                    && (def.native() == def.name() || matches!(def.returned(), Type::Text(_)))
                    && !data.wasm_bridge_routes.contains_key(def.native())
                {
                    continue;
                }
                declared_natives.insert(def.native().to_string());
                // Build the C-ABI signature from loft parameter types.
                use std::fmt::Write as _;
                let mut params = String::new();
                for attr in def.attributes() {
                    if attr.name.starts_with("__") {
                        continue;
                    }
                    if !params.is_empty() {
                        params.push_str(", ");
                    }
                    let name = sanitize(&attr.name);
                    match &attr.typedef {
                        Type::Text(_) => params.push_str("ptr: *const u8, len: usize"),
                        Type::Vector(elem_tp, _) => {
                            let elem = Self::vector_elem_rust_type(elem_tp);
                            let _ = write!(params, "ptr: *const {elem}, count: u32");
                        }
                        // Post-2c round 10c: wide Type::Integer (former Type::Long)
                        // passes as i64 — range > i32::MAX can't fit in i32.
                        Type::Integer(s) if s.is_wide() => {
                            let _ = write!(params, "{name}: i64");
                        }
                        Type::Float => {
                            let _ = write!(params, "{name}: f64");
                        }
                        Type::Single => {
                            let _ = write!(params, "{name}: f32");
                        }
                        // @PLN17: cdylib exports use the u8 boolean storage form
                        // (generated via rust_type); the extern decl must match.
                        Type::Boolean => {
                            let _ = write!(params, "{name}: u8");
                        }
                        _ => {
                            let _ = write!(params, "{name}: i32");
                        }
                    }
                }
                let ret = match def.returned() {
                    Type::Void => String::new(),
                    // Post-2c round 10c: wide Type::Integer returns as i64.
                    Type::Integer(s) if s.is_wide() => " -> i64".to_string(),
                    Type::Integer(_) | Type::Character => " -> i32".to_string(),
                    Type::Float => " -> f64".to_string(),
                    Type::Single => " -> f32".to_string(),
                    Type::Boolean => " -> u8".to_string(), // @PLN17: u8 storage form
                    _ => " -> i32".to_string(),
                };
                writeln!(w, "    safe fn {}({params}){ret};", def.native())?;
            }
            writeln!(w, "}}")?;
        } else if native_cabi {
            // Host-native backend (NATIVE.md § Resolution: separate the API
            // id from the Rust part).  Declare each reachable `#native`
            // package function as an `extern "C"` symbol; the cdylib `.so`
            // is linked by C-ABI (`add_native_extern_flags`), so the
            // package's Rust crate graph stays sealed inside the `.so` and
            // the shared-dep `StableCrateId` collision class cannot arise.
            // No `extern crate <pkg>`.  The signature mirrors exactly what
            // `output_native_direct_call` marshals (store handle first,
            // text → ptr+len, vector → ptr+count, ref → LoftRef, scalar
            // widths per `is_wide`), so the unqualified C-ABI call is
            // type-correct.  The opaque FFI types are named through loft's
            // own `loft_ffi` (`--extern loft_ffi`) — the only copy left in
            // the consumer now the cdylib's copy is sealed away.
            use std::fmt::Write as _;
            writeln!(w, "unsafe extern \"C\" {{")?;
            let mut declared = HashSet::new();
            for d_nr in 0..data.definitions() {
                let def = data.def(d_nr);
                // Only a body-less `#native` fn (`code() == Null`) is called
                // as a native symbol.  A fn WITH a loft body is compiled
                // inline as a regular `n_*` fn and the call resolves to that.
                // A pure-loft *package* fn carries a `loft_shared_*` bridge
                // name in `native()` (for the interpreter's shared-store
                // dispatch), but the native whole-program backend inlines it;
                // declaring it here would emit a dead, wrong-ABI extern.  This
                // matches the call branch, which emits a native call only when
                // `code() == Null` (see the `else if *def.code() == Value::Null`
                // arm in the fn emitter).
                if def.native().is_empty() || *def.code() != Value::Null {
                    continue;
                }
                // Declare only natives belonging to a `[native] crate` package
                // we link by C-ABI — those have a `native_symbol_crates` entry.
                // A stem/dlopen native (`[library] native = "..."`, resolved at
                // runtime via `load_all`) is NOT linked here; declaring it
                // would force an undefined-symbol link error instead of leaving
                // it on its own dispatch path (the call branch keeps such a
                // symbol off the C-ABI route too).
                if !data.native_symbol_crates.contains_key(def.native()) {
                    continue;
                }
                // No reachability filter here: the fn emitter emits a wrapper
                // (a native call) for EVERY body-less native def in range —
                // even ones the user never calls — so every such symbol needs
                // a decl or the wrapper fails to compile (E0425).  Declaring a
                // genuinely-unreferenced one is a harmless dead `extern` (no
                // symbol reference is emitted, so the linker never needs it).
                if !declared.insert(def.native().to_string()) {
                    continue;
                }
                // A store handle is the first parameter when the fn writes
                // the store: a heap-typed arg (Reference / Vector / data-enum /
                // sorted / hash / index / spatial) the cdylib reads or mutates,
                // or a Vector/Reference return it allocates.  `heap_dep()` is the
                // canonical set of heap types — the same union the interpreter
                // marshals as a `LoftRef` handle (`compute_sig`); reuse it so
                // both backends pass the identical ABI.
                let returns_ref = matches!(
                    def.returned().base(),
                    Type::Vector(_, _) | Type::Reference(_, _)
                );
                let has_ref_arg = def
                    .attributes()
                    .iter()
                    .any(|a| !a.name.starts_with("__") && a.typedef.base().heap_dep().is_some());
                let mut sig = String::new();
                if returns_ref || has_ref_arg {
                    sig.push_str("store: loft_ffi::LoftStore");
                }
                for attr in def.attributes() {
                    if attr.name.starts_with("__") {
                        continue;
                    }
                    if !sig.is_empty() {
                        sig.push_str(", ");
                    }
                    let n = sanitize(&attr.name);
                    // A heap-typed arg is passed by `LoftRef` handle — matched
                    // first so Vector and the keyed collections share the
                    // Reference convention (the interpreter's `ArgT::Ref`/`Vec`).
                    // ABI classification is layout-based, and `Optional(τ)` shares
                    // τ's sentinel layout — classify the peeled type throughout.
                    if attr.typedef.base().heap_dep().is_some() {
                        let _ = write!(sig, "{n}: loft_ffi::LoftRef");
                        continue;
                    }
                    match attr.typedef.base() {
                        Type::Text(_) => {
                            let _ = write!(sig, "{n}_ptr: *const u8, {n}_len: usize");
                        }
                        // A plain loft `integer` is 64-bit at the package C-ABI —
                        // the SAME judgment as the interpreter marshal
                        // (`extensions::compute_sig`, @P370): only an explicit
                        // narrow integer (`forced_size`) is 4 bytes.  Deciding by
                        // `is_wide()` here declared `i32` against an i64 cdylib,
                        // silently truncating i64 values (the null sentinel → 0).
                        Type::Integer(s) if s.forced_size.is_none() => {
                            let _ = write!(sig, "{n}: i64");
                        }
                        Type::Float => {
                            let _ = write!(sig, "{n}: f64");
                        }
                        Type::Single => {
                            let _ = write!(sig, "{n}: f32");
                        }
                        // `output_native_direct_call` passes `var != 0` (a
                        // `bool`) for a boolean arg; declare it `bool`.
                        Type::Boolean => {
                            let _ = write!(sig, "{n}: bool");
                        }
                        _ => {
                            let _ = write!(sig, "{n}: i32");
                        }
                    }
                }
                let ret = match def.returned().base() {
                    Type::Void => String::new(),
                    Type::Integer(s) if s.forced_size.is_none() => " -> i64".to_string(),
                    Type::Integer(_) | Type::Character => " -> i32".to_string(),
                    Type::Float => " -> f64".to_string(),
                    Type::Single => " -> f32".to_string(),
                    // @PLN26 phase 0.2 — declare `u8` (loft's boolean storage
                    // form, 0/1/255), NOT `bool`: a cdylib returning a u8 that
                    // isn't 0/1, read back through a `bool` return, is UB.  The
                    // call's `(…) as u8` becomes identity.  (The boolean *arg*
                    // stays `bool` — `output_native_direct_call` passes
                    // `var != 0`, always 0/1, valid at the 1-byte C-ABI.)
                    Type::Boolean => " -> u8".to_string(),
                    Type::Text(_) => " -> loft_ffi::LoftStr".to_string(),
                    Type::Vector(_, _) | Type::Reference(_, _) => {
                        " -> loft_ffi::LoftRef".to_string()
                    }
                    _ => " -> i32".to_string(),
                };
                // The C-ABI symbol name can collide with the generated
                // wrapper fn: a bare `#native` defaults the symbol to the
                // fn's own `n_<name>` (e.g. `load_png` → wrapper `n_load_png`
                // AND symbol `n_load_png`).  Declare under a `__cabi_`-prefixed
                // local alias and bind the real symbol via `#[link_name]`, so
                // the extern never shadows the wrapper (E0428).
                //
                // loft#907: the linked symbol is the one the LIBRARY says
                // implements it, which for a remapped binding is not the
                // `#native` string.  The alias stays keyed on the `#native`
                // string so call sites are unaffected.
                writeln!(
                    w,
                    "    #[link_name = \"{}\"]",
                    data.link_symbol(def.native())
                )?;
                writeln!(w, "    fn __cabi_{}({sig}){ret};", def.native())?;
            }
            writeln!(w, "}}")?;
        } else {
            // Emit extern crate declarations only for the native packages the
            // emitted (reachable) code actually calls (#307).  A library cdylib
            // built inside a multi-package program (the viewer: markdown +
            // server + web) must not declare unrelated packages: no `--extern`
            // is supplied for them (E0463), and two loft-ffi package rlibs
            // cannot link into one cdylib anyway (each exports `#[no_mangle]
            // loft_register_v1` → duplicate symbol).  An empty `reachable` is
            // the whole-program fallback — keep every package.
            let needed: Option<HashSet<&String>> = if reachable.is_empty() {
                None
            } else {
                Some(
                    reachable
                        .iter()
                        .filter_map(|&d| data.native_symbol_crates.get(data.def(d).native()))
                        .collect(),
                )
            };
            for (crate_name, _) in &data.native_packages {
                let ident = crate_name.replace('-', "_");
                if needed.as_ref().is_none_or(|n| n.contains(&ident)) {
                    writeln!(w, "extern crate {ident};")?;
                }
            }
        }
        writeln!(w, "use loft::database::Stores;")?;
        writeln!(w, "use loft::keys::{{DbRef, Str, Key, Content}};")?;
        writeln!(w, "use loft::ops;")?;
        writeln!(w, "use loft::vector;")?;
        writeln!(w, "use loft::hash;")?;
        writeln!(w, "use loft::tree;")?;
        writeln!(w, "use loft::codegen_runtime;")?;
        writeln!(w, "use loft::codegen_runtime::*;")?;

        // @PLN24 arc C — one typed `extern "C"` declaration per `#c` symbol.
        //
        // This is where the plan's invariant pays off: the DECLARED widths go
        // straight into the extern, so rustc performs the ABI truncation and
        // the call site's `as i64` then extends correctly.  A caller that read
        // the same 32-bit return as a bare `u64` — which is what a
        // signature-blind trampoline does — turns -1 into 4294967295.  Nothing
        // here re-derives a width from the loft types; they cannot express one.
        //
        // No reachability filter, matching the `#native` block above: the fn
        // emitter writes a wrapper for every body-less def, so every symbol
        // needs a declaration or the wrapper fails to compile.  An unreferenced
        // extern is dead, not a link error.
        //
        // @PLN24 arc E — except on a wasm target, where nothing is declared and
        // nothing is resolved.  Both halves matter and they fail differently: an
        // `extern "C"` that IS satisfied by the wasm sysroot's own libc links
        // against the wrong data model (wasm32 is ILP32 — `long`, `size_t` and
        // every pointer are 32 bits, against the host's 64) and traps at the
        // call, while the lazy resolver names `loft::c_call`, which is gated
        // behind `native-extensions` and so is not in a wasm build at all.  The
        // second one fired for a binding the program never called, so this skip
        // is what keeps an unused `#c` declaration from rejecting an otherwise
        // valid wasm program.  A REACHABLE call is refused by name, at the call
        // site (`output_c_direct_call`).
        // Emitted AFTER the tables below, which is where it has always sat: the
        // wrappers read `__C_LIBS` / `__C_LIB_SYMS`, and keeping the order means
        // a non-wasm target's generated Rust is unchanged, byte for byte, by
        // arc E.
        let mut lazy = String::new();
        if !no_c_abi {
            let target = crate::c_signature::CTarget::host();
            let mut declared_c: HashSet<String> = HashSet::new();
            let mut block = String::new();
            // @PLN24 arc G — a symbol from a package that declares an OPTIONAL
            // library cannot be declared `extern "C"`: an extern that is CALLED
            // is an undefined reference, and with the library off the link line
            // the program fails to build (measured — `rust-lld: unable to find
            // library`, even for a call the program never reaches).  It gets a
            // resolving wrapper instead, below.
            for d_nr in 0..data.definitions() {
                let def = data.def(d_nr);
                if def.c_sig.is_empty() || *def.code() != Value::Null {
                    continue;
                }
                let Some(Ok(sig)) = crate::c_signature::of(data, d_nr, target) else {
                    continue; // the parse error was already reported at the declaration
                };
                if !declared_c.insert(sig.symbol.clone()) {
                    continue;
                }
                let params = sig
                    .params
                    .iter()
                    .enumerate()
                    .map(|(i, t)| format!("a{i}: {}", t.rust_type()))
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret = match sig.ret {
                    crate::c_signature::CType::Void => String::new(),
                    ref t => format!(" -> {}", t.rust_type()),
                };
                use std::fmt::Write as _;
                if data.c_symbol_is_lazy(&def.position().file) {
                    let fn_ty = format!(
                        "unsafe extern \"C\" fn({}){ret}",
                        sig.params
                            .iter()
                            .map(crate::c_signature::CType::rust_type)
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    let args = (0..sig.params.len())
                        .map(|i| format!("a{i}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    // An `unsafe fn` under the SAME name the call site already
                    // writes, so nothing changes at the call: the emission of a
                    // `#c` call stays one shape whether or not it is lazy.
                    //
                    // The pointer is transmuted to the DECLARED signature, never
                    // called through a `u64` trampoline — that is arc C's whole
                    // gain (rustc applies the C widths at the ABI), and a lazy
                    // call that dropped it would reintroduce `atoi("-1")`
                    // answering 4294967295.
                    let _ = writeln!(lazy, "#[inline]");
                    let _ = writeln!(lazy, "unsafe fn __c_{}({params}){ret} {{", sig.symbol);
                    let _ = writeln!(
                        lazy,
                        "  static P: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();"
                    );
                    let _ = writeln!(
                        lazy,
                        "  match P.get_or_init(|| loft::c_call::resolve_native({:?}, __C_LIBS, __C_LIB_SYMS).map(|p| p as usize)) {{",
                        sig.symbol
                    );
                    let _ = writeln!(
                        lazy,
                        "    Some(p) => unsafe {{ std::mem::transmute::<usize, {fn_ty}>(*p)({args}) }},"
                    );
                    let _ = writeln!(
                        lazy,
                        "    None => panic!(\"{{}}\", loft::c_call::missing_symbol_message({:?})),",
                        sig.symbol
                    );
                    let _ = writeln!(lazy, "  }}");
                    let _ = writeln!(lazy, "}}");
                    continue;
                }
                // Aliased like the `#native` block, and for the same reason: a
                // C symbol may share the wrapper fn's name (E0428).
                let _ = writeln!(block, "    #[link_name = \"{}\"]", sig.symbol);
                let _ = writeln!(block, "    fn __c_{}({params}){ret};", sig.symbol);
            }
            if !block.is_empty() {
                writeln!(w, "unsafe extern \"C\" {{")?;
                write!(w, "{block}")?;
                writeln!(w, "}}")?;
            }
        }
        // The tables a compiled program needs because it never runs
        // `c_call::register`: which libraries may be opened on demand, and
        // which `#c` symbols each is expected to provide (the availability
        // query's symbol-granular half).
        //
        // EVERY declared library is listed, not just the optional ones: with
        // its symbols resolved lazily a required library may have no
        // undefined reference left to keep its `DT_NEEDED` entry alive, and
        // re-opening one that is already loaded costs nothing.
        //
        // Emitted UNCONDITIONALLY, even empty, and on EVERY target including the
        // two wasm ones. `c_library_available`'s `#rust` body reads them, and
        // that body is compiled into whichever unit calls it — the main binary,
        // an auto-built package cdylib (which links its own copy of loft and
        // therefore has its own copy of these tables), or a wasm module.
        // Emitting them only "when needed" is what made the same query answer
        // true from a program and false from inside the package that declared
        // the library; emitting them only on non-browser targets is what made
        // `c_library_available` — the very question a library is told to ask
        // before calling into C — fail to compile under `--html` (@PLN24 arc E).
        writeln!(w, "static __C_LIBS: &[(&str, &str)] = &[")?;
        for lib in &data.c_libraries {
            writeln!(w, "    ({:?}, {:?}),", lib.name, lib.pkg_dir)?;
        }
        writeln!(w, "];")?;
        writeln!(w, "static __C_LIB_SYMS: &[(&str, &[&str])] = &[")?;
        // The table lives in `c_call`, which exists only with
        // `native-extensions`. Without it nothing can be `dlopen`ed, so the
        // right table is the empty one rather than a build failure.
        #[cfg(feature = "native-extensions")]
        for (lib, syms) in crate::c_call::library_symbol_table(data) {
            write!(w, "    ({lib:?}, &[")?;
            for s in &syms {
                write!(w, "{s:?}, ")?;
            }
            writeln!(w, "]),")?;
        }
        writeln!(w, "];")?;
        if !lazy.is_empty() {
            write!(w, "{lazy}")?;
        }
        Ok(())
        // @PLAN12 phase 3.5a (2026-05-24) — removed the `mod external {…}`
        // shim that wrapped cr_rand_int / cr_rand_seed.  No `#rust
        // "external::rand_*"` consumers remain in default/ or lib/random/;
        // random's #native annotations now route through the cdylib path
        // (loft::native_call::build_store + cdylib call).
    }

    /// Use this as the main entry point for native Rust code generation.
    ///
    /// # Errors
    /// Returns an error if any write action to `w` fails.
    pub fn output_native(
        &mut self,
        w: &mut dyn Write,
        from: u32,
        till: u32,
    ) -> std::io::Result<()> {
        // Buffer then scrub (see `scrub_generated_crate_refs`): the full
        // program source passes the `crate::rpc::`/`crate::store::` invariant
        // exactly once, regardless of which template path emitted it.
        let mut buf: Vec<u8> = Vec::new();
        self.output_native_emit(&mut buf, from, till)?;
        w.write_all(&scrub_generated_crate_refs(&buf))
    }

    fn output_native_emit(
        &mut self,
        w: &mut dyn Write,
        from: u32,
        till: u32,
    ) -> std::io::Result<()> {
        self.dup_fn_names = duplicate_fn_names(self.data);
        // P269 — populate reachability from `n_main` so the unimplemented-
        // native check (`output_function`'s `todo!()`-emit branches) can
        // distinguish "reachable AND unimpl" (compile_error!, fail at
        // build time per the "fail at startup, not runtime" principle)
        // from "unreachable + unimpl" (harmless `todo!()` shim).  Without
        // this, the reachability set was empty under `--native` (only
        // populated by `output_native_reachable`), so EVERY unimpl native
        // would have to compile-error or NONE could.  Now the two emit
        // paths agree on what's actually live in the program.
        let main_nr = self.data.def_nr("n_main");
        if main_nr < self.data.definitions() {
            let roots = self.roots_with_lazy_driver(&[main_nr]);
            self.reachable = reachable_functions(self.data, &roots);
        } else {
            // No `n_main` (test-script style with `fn p203_*` as the
            // top-level fn).  Walk reachability from EVERY user-source
            // function so the compile_error check has accurate data.
            // Without this, the `is_empty()` fallback in
            // `output_function` treats every unimplemented stub as
            // reachable and emits compile_error, breaking test
            // scripts that don't define a main.
            //
            // "User source" = position.file outside `default/` and
            // `lib/`.  Imprecise (it includes any user file, not just
            // the entry script) but safe — the worst case is a few
            // false-positive reachable defs that emit compile_error
            // when they could've been todo!(), still better than the
            // pre-fix behaviour where EVERYTHING was treated as
            // reachable.
            let mut entries: Vec<u32> = Vec::new();
            for d in 0..self.data.definitions() {
                let def = self.data.def(d);
                if !matches!(def.def_type(), DefType::Function) {
                    continue;
                }
                let pos_file = &def.position().file;
                // Match `/default/` and `/lib/` segments under EITHER path
                // separator — Windows uses `\` so a bare `/default/`
                // substring check misses `default\01_code.loft` and
                // misclassifies every stdlib fn as user source, which
                // pulls the entire stdlib into the reachable set and
                // raises P269 compile_error for unimplemented natives
                // (n_parallel_queue_fn / n_parallel_buf_get_fn / ...).
                // Linux/macOS only ever produce `/` separators so the
                // original check happened to work; Windows surfaced
                // the gap in PR #212 CI.
                let normalised: String = pos_file
                    .chars()
                    .map(|c| if c == '\\' { '/' } else { c })
                    .collect();
                if pos_file.is_empty()
                    || normalised.contains("/default/")
                    || normalised.contains("/lib/")
                {
                    continue;
                }
                entries.push(d);
            }
            if !entries.is_empty() {
                let roots = self.roots_with_lazy_driver(&entries);
                self.reachable = reachable_functions(self.data, &roots);
            }
        }
        Self::emit_file_header(
            w,
            self.data,
            self.wasm_browser,
            self.no_c_abi(),
            self.native_cabi,
            &self.reachable,
        )?;
        writeln!(w, "fn init(cell: &std::cell::UnsafeCell<Stores>) {{")?;
        writeln!(
            w,
            "    let db: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
        self.output_init(w, from, till)?;
        writeln!(w, "    db.finish();")?;
        // Mirror `compile::build_const_vectors` so module-scope `const`
        // vectors (`const NUMS = [10, 20, 30]`) populate `db.const_refs`
        // before `n_main` runs.  Without this, `OpConstRef(<d_nr>)`
        // indexes into an empty Vec and panics.  @P275 fix.
        self.emit_const_vectors(w, till)?;
        self.emit_lazy_fetch_registration(w, till)?;
        writeln!(w, "}}\n")?;
        self.output_functions(w, from, till, None)?;
        self.emit_main_bootstrap(w, till)
    }

    /// Like `output_native`, but only emits functions reachable from `entry_defs`.
    /// Stdlib functions outside `[from, till)` are included if they are transitively
    /// called.  Use this for per-test files so they are self-contained without
    /// emitting the entire stdlib.
    ///
    /// # Errors
    /// Returns an error if any write action to `w` fails.
    pub fn output_native_reachable(
        &mut self,
        w: &mut dyn Write,
        _from: u32,
        till: u32,
        entry_defs: &[u32],
    ) -> std::io::Result<()> {
        // Buffer then scrub the assembled source (see
        // `scrub_generated_crate_refs`), then write to the real `w`.
        let mut buf: Vec<u8> = Vec::new();
        self.emit_native_reachable_body(&mut buf, till, entry_defs)?;
        // Emit a Rust entry point that bootstraps the loft `main` function, if present.
        if (0..till).any(|d| self.data.def(d).name() == "n_main") {
            if self.wasm_browser {
                self.emit_wasm_start(&mut buf)?;
            } else {
                // Native binary: thread std::env::args() into Stores.user_args
                // so the loft `arguments()` builtin returns the program's
                // CLI args.  Skip [0] (binary path).  Mirrors what
                // src/main.rs does for the interpreter path
                // (state.database.user_args.clone_from(&user_args)).
                // @PLAN37 phase 10.3 fix.
                self.emit_native_main(&mut buf)?;
            }
        }
        w.write_all(&scrub_generated_crate_refs(&buf))
    }

    /// Does the emitted program dispatch `par` / `par_fold`?
    ///
    /// Meaningful once the reachable set exists (after
    /// [`output_native_reachable`](Self::output_native_reachable)).  `--html`
    /// asks so a program that never runs anything in parallel is not made to
    /// carry browser threading: a threaded page needs a nightly toolchain to
    /// build and a cross-origin-isolated host to serve (@PLN117 opt-out).
    #[must_use]
    pub fn uses_parallel(&self) -> bool {
        self.reachable
            .iter()
            .any(|d| self.data.def(*d).name().starts_with("n_parallel_"))
    }

    /// Emit the reachable native program **without any `fn main()` / `loft_start`
    /// bootstrap**, even when an `n_main` exists in `data`.
    ///
    /// Use this whenever the caller owns the crate's entry point.
    /// [`Self::output_native_reachable`] decides to emit the bootstrap by scanning
    /// every definition for the name `n_main`, which finds an `n_main` belonging to
    /// a LINKED PACKAGE just as readily as one in the program being compiled — so
    /// the caller, not that scan, has to be the authority when it supplies its own
    /// `main`.  Two callers do:
    ///
    ///   * @PLN11 Arc N library cdylibs, where the `n_main` belongs to the
    ///     consuming script and isn't reachable from the library's exports (the
    ///     bootstrap would reference an undefined `n_main`);
    ///   * the `loft test --native` harness, which appends its own `main` calling
    ///     each `test_*` function — for a package whose `src/` entry is also a
    ///     runnable CLI, both mains landed in one crate and rustc rejected it with
    ///     E0428 (#621).
    ///
    /// # Errors
    /// Returns any `io::Error` from writing to `w`.
    pub fn output_native_no_bootstrap(
        &mut self,
        w: &mut dyn Write,
        _from: u32,
        till: u32,
        entry_defs: &[u32],
    ) -> std::io::Result<()> {
        // Buffer then scrub the assembled source — see
        // `scrub_generated_crate_refs`.
        let mut buf: Vec<u8> = Vec::new();
        self.emit_native_reachable_body(&mut buf, till, entry_defs)?;
        w.write_all(&scrub_generated_crate_refs(&buf))
    }

    /// @PLN11 Arc N — emit the reachable native program as a **library** cdylib.
    /// See [`Self::output_native_no_bootstrap`], which this names for the library case.
    ///
    /// # Errors
    /// Returns any `io::Error` from writing to `w`.
    pub fn output_native_library(
        &mut self,
        w: &mut dyn Write,
        from: u32,
        till: u32,
        entry_defs: &[u32],
    ) -> std::io::Result<()> {
        self.output_native_no_bootstrap(w, from, till, entry_defs)
    }

    /// Shared prelude of [`Self::output_native_reachable`] /
    /// [`Self::output_native_library`]: header + `init` (all types) + const
    /// vectors + only the reachable functions.  The two callers differ only in
    /// whether they then emit the `main` bootstrap.
    fn emit_native_reachable_body(
        &mut self,
        w: &mut dyn Write,
        till: u32,
        entry_defs: &[u32],
    ) -> std::io::Result<()> {
        self.dup_fn_names = duplicate_fn_names(self.data);
        let roots = self.roots_with_lazy_driver(entry_defs);
        let reachable = reachable_functions(self.data, &roots);
        self.reachable.clone_from(&reachable);
        Self::emit_file_header(
            w,
            self.data,
            self.wasm_browser,
            self.no_c_abi(),
            self.native_cabi,
            &reachable,
        )?;
        writeln!(w, "fn init(cell: &std::cell::UnsafeCell<Stores>) {{")?;
        writeln!(
            w,
            "    let db: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
        // Register ALL types (0..till) so runtime type IDs match compile-time IDs.
        self.output_init(w, 0, till)?;
        writeln!(w, "    db.finish();")?;
        // Initiative 03 Phase 3b: emit code to build CONST_STORE
        // vectors and populate `db.const_refs` — mirrors the
        // interpreter path in `compile::build_const_vectors`.
        self.emit_const_vectors(w, till)?;
        self.emit_lazy_fetch_registration(w, till)?;
        writeln!(w, "}}\n")?;
        // Emit only reachable functions across the full definition range.
        self.output_functions(w, 0, till, Some(&reachable))
    }

    /// Emit a Rust `fn main()` bootstrap if the program defines a loft `main` function.
    fn emit_main_bootstrap(&self, w: &mut dyn Write, till: u32) -> std::io::Result<()> {
        let main_nr = self.data.def_nr("n_main");
        if main_nr < till {
            self.emit_native_main(w)?;
        }
        Ok(())
    }

    /// The arguments the generated entry call must supply beyond `&cell`,
    /// returned as `(prelude, args)`: statements to emit before the call, and
    /// text to append inside its parentheses.  Both are empty for a plain
    /// `fn main()`, so the common path stays byte-identical.
    ///
    /// #629: every entry template hardcoded `n_main(&cell)`, but `n_main` is
    /// emitted from its DECLARATION, so any entry taking more than the cell
    /// produced a rustc `E0061` cascade instead of a loft program.  Two shapes
    /// reach that: a declared `fn main(args: vector<text>)`, and the HIDDEN
    /// return buffer `ref_return` promotion adds to a heap-returning entry.
    ///
    /// This is the native twin of the interpreter's `State::execute_argv`, and
    /// it follows the same split — so keep the two in step:
    ///
    /// - a declared (non-hidden) `vector<text>` parameter receives argv;
    /// - a hidden VECTOR attr must be a REAL allocated buffer, because a vector
    ///   body writes straight into the caller's record (`clear_vector` /
    ///   `pre_alloc_vector`) and never allocates one itself;
    /// - a hidden struct / data-enum attr takes the null sentinel, which its
    ///   body turns into a store via its own `OpDatabase`.
    ///
    /// The same vector-vs-reference split is already spelled out for direct
    /// callers in `generation/emit.rs` (the `vec_hbuf_tp` comment) and
    /// `generation/dispatch.rs`; the entry is simply another caller.
    fn entry_call_extra_args(&self) -> (String, String) {
        use std::fmt::Write as _;
        let d_nr = self.data.def_nr("n_main");
        if d_nr == u32::MAX {
            return (String::new(), String::new());
        }
        let mut prelude = String::new();
        let mut args = String::new();
        for (i, a) in self.data.def(d_nr).attributes().iter().enumerate() {
            if !a.hidden {
                // The only declared entry parameter the runtime supplies is argv.
                if matches!(a.typedef.base(), Type::Vector(_, _)) {
                    let v = format!("__entry_argv_{i}");
                    let _ = writeln!(
                        prelude,
                        "    let {v}: DbRef = {{ let stores: &mut Stores = unsafe {{ &mut *cell.get() }}; let a = stores.user_args.clone(); stores.text_vector(&a) }};"
                    );
                    let _ = write!(args, ", {v}");
                }
                continue;
            }
            match a.typedef.base() {
                Type::Vector(elm_tp, _) => {
                    let elm_name = elm_tp.name(self.data);
                    let tp = self.data.name_type(&format!("main_vector<{elm_name}>"), 0);
                    if tp == u16::MAX {
                        // No wrapper registered — the sentinel is what a
                        // pre-#629 build passed, and the body's null guards
                        // make it a no-op rather than a crash.
                        args.push_str(", DbRef::NULL");
                        continue;
                    }
                    let v = format!("__entry_ret_{i}");
                    let _ = write!(
                        prelude,
                        "    let mut {v}: DbRef = {{ let stores: &mut Stores = unsafe {{ &mut *cell.get() }}; stores.null_named(\"{v}\") }};\n    {v} = OpDatabase(&cell, {v}, {tp}_i32);\n"
                    );
                    let _ = write!(args, ", {v}");
                }
                Type::Reference(_, _) | Type::Enum(_, true, _) => {
                    args.push_str(", DbRef::NULL");
                }
                // A TEXT return is promoted the same way, but its buffer is a
                // Rust `String` the caller owns rather than a store record —
                // the signature takes `&mut String`, so the entry has to own
                // one too (`let _pre = { …; &mut var___work_1 }` is what an
                // ordinary call site emits).  `base()` peels `Optional`, NOT
                // `RefVar`, so this arm must match the wrapper itself.
                Type::RefVar(inner) if matches!(inner.base(), Type::Text(_)) => {
                    let v = format!("__entry_text_{i}");
                    let _ = writeln!(prelude, "    let mut {v}: String = String::new();");
                    let _ = write!(args, ", &mut {v}");
                }
                _ => {}
            }
        }
        (prelude, args)
    }

    /// @PLN18 08-S2 — the ONE native `fn main()` template (both program
    /// emission paths).  Under `LOFT_LIVE_FLIP=1` the world comes from
    /// `live_dispatch::boot_stores` (a full parse of the same sources, so the
    /// parked interpreter and the compiled code share one id-compatible
    /// world) and `init` is skipped — the parse already seeded it.  The leak
    /// check is also skipped live: the parked interpreter's machinery stores
    /// are not program leaks.
    /// loft#950 — the channel a browser page arms a `LOFT_*` switch through.
    ///
    /// `std::env` on `wasm32-unknown-unknown` is a stub, so a page could set none of the
    /// 47 switches `keys.rs` reads, and the store guard's own advice — *"run with
    /// LOFT_STRICT_STORES=1 to name the free"* — named something the target cannot do.
    /// Every instrument this issue needed had to be built twice for that reason.
    ///
    /// The module owns the buffer and the host fills it, the same shape the asyncify
    /// region uses: the page writes `NAME=VALUE` lines from its query string and calls
    /// `loft_env_commit(len)` BEFORE `loft_start`.  `keys.rs` memoizes each switch on
    /// first read, so anything installed after the program starts would be ignored by
    /// whichever switch had already been asked — committing before entry is what makes
    /// the setting mean what it says.
    ///
    /// Deliberately NOT a generation-time bake.  Baking works (it is how
    /// `LOFT_PROGRAM_RELATIVE` travels) but costs a full rebuild per setting, and a
    /// rebuild per diagnostic is exactly what made the browser expensive to debug.
    /// `index.html?LOFT_STRICT_STORES=1` costs a reload.
    fn emit_page_env_region(w: &mut dyn Write) -> std::io::Result<()> {
        writeln!(
            w,
            "\n// loft#950 — switch settings the host writes in before `loft_start`.\n\
             const LOFT_ENV_CAP: usize = 4096;\n\
             static mut LOFT_ENV_BUF: [u8; LOFT_ENV_CAP] = [0; LOFT_ENV_CAP];\n\
             \n\
             /// Where to write the `NAME=VALUE` lines.\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn loft_env_buf() -> u32 {{\n    \
             (&raw const LOFT_ENV_BUF).cast::<u8>() as usize as u32\n\
             }}\n\
             \n\
             /// How many bytes that buffer holds.\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn loft_env_cap() -> u32 {{\n    \
             LOFT_ENV_CAP as u32\n\
             }}\n\
             \n\
             /// Adopt the first `len` bytes as this program's switch settings.  A length\n\
             /// past the buffer is clamped rather than refused: the host wrote what it\n\
             /// wrote, and truncating costs a setting where trusting it costs memory.\n\
             /// Invalid UTF-8 is ignored, so a mangled query string leaves the program\n\
             /// running with no switches instead of not running.\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn loft_env_commit(len: u32) {{\n    \
             let n = (len as usize).min(LOFT_ENV_CAP);\n    \
             let bytes = unsafe {{ &(&raw const LOFT_ENV_BUF).cast::<[u8; LOFT_ENV_CAP]>().read()[..n] }};\n    \
             if let Ok(text) = core::str::from_utf8(bytes) {{\n        \
             loft::keys::install_page_env(text);\n    \
             }}\n\
             }}"
        )
    }

    /// loft#950 — reserve the asyncify save region INSIDE the module, and export where
    /// it is.
    ///
    /// `wasm-opt --asyncify` spills the whole wasm stack into a caller-chosen buffer on
    /// every suspend and reads it back on resume, so that buffer has to be memory nothing
    /// else owns.  The JS driver used to pick the address itself: `__heap_base` when the
    /// module exported it, and the literal `65536` when it did not.  It never did — that
    /// export rides in `WASM_THREAD_FLAGS`, which only the THREADED `--html` link passes
    /// — so every ordinary page took the fallback, and `65536` is not in the heap at all.
    /// `__stack_pointer` starts at `0x100000`, so the shadow stack is `[0, 0x100000)`
    /// growing DOWN and the save region sat 966,648 bytes into it: a page whose stack
    /// ever reached that depth had asyncify writing its saved frames over live locals,
    /// and reading live locals back as saved frames.
    ///
    /// A static owns its address instead.  The allocator cannot hand it out, the shadow
    /// stack cannot grow into it, and there is no arithmetic in JS to get wrong — the
    /// driver asks the module and the module answers.
    ///
    /// The trailing CANARY is the other half.  Moving the buffer says nothing about it
    /// being big enough, and an overflow of the old one was silent for the same reason
    /// the misplacement was: it corrupts whatever is next and the failure surfaces
    /// somewhere else entirely.  `loft_asyncify_ok` reports it, so a stack deeper than
    /// the region gets a sentence instead of a wrong answer somewhere downstream.
    ///
    /// One buffer per MODULE, which is what the `__heap_base` scheme was too: in a
    /// threaded page every worker shares this linear memory, so two suspending at once
    /// would still share the region.  Not a regression, and not this fix's job.
    fn emit_asyncify_region(w: &mut dyn Write) -> std::io::Result<()> {
        writeln!(
            w,
            "\n// loft#950 — the asyncify save region, owned by the module rather than\n\
             // chosen by the JS driver.  `loft_asyncify_data` is the 8-byte control\n\
             // struct asyncify takes; the save area follows it, and a canary follows\n\
             // that so an overflow is reported instead of silently overwriting the\n\
             // next static.\n\
             const LOFT_ASYNCIFY_SAVE: usize = 16384;\n\
             const LOFT_ASYNCIFY_CANARY: [u8; 8] = [0x4c, 0x4f, 0x46, 0x54, 0x39, 0x35, 0x30, 0x21];\n\
             #[repr(C, align(16))]\n\
             struct LoftAsyncifyRegion {{\n    \
             ctrl: [u8; 8],\n    \
             save: [u8; LOFT_ASYNCIFY_SAVE],\n    \
             canary: [u8; 8],\n\
             }}\n\
             static mut LOFT_ASYNCIFY_REGION: LoftAsyncifyRegion = LoftAsyncifyRegion {{\n    \
             ctrl: [0; 8],\n    \
             save: [0; LOFT_ASYNCIFY_SAVE],\n    \
             canary: LOFT_ASYNCIFY_CANARY,\n\
             }};\n\
             \n\
             /// Address of the 8-byte asyncify control struct.  The save area is the\n\
             /// `loft_asyncify_size()` bytes after it.\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn loft_asyncify_data() -> u32 {{\n    \
             (&raw const LOFT_ASYNCIFY_REGION).cast::<u8>() as usize as u32\n\
             }}\n\
             \n\
             /// Size of the save area, so the driver never carries its own copy of it.\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn loft_asyncify_size() -> u32 {{\n    \
             LOFT_ASYNCIFY_SAVE as u32\n\
             }}\n\
             \n\
             /// False once a suspend has spilled more than the save area holds.\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn loft_asyncify_ok() -> i32 {{\n    \
             let canary = unsafe {{ (&raw const LOFT_ASYNCIFY_REGION.canary).read() }};\n    \
             i32::from(canary == LOFT_ASYNCIFY_CANARY)\n\
             }}"
        )
    }

    /// @PLN98 P3.4 — emit the browser (`--html`) `loft_start` export.  WASM has no
    /// argv (so `arguments()` is `[]`) and no filesystem.  Two shapes gated on the
    /// `--debug` opt-in (`emit_live`):
    /// - **production client** (default): a plain `Stores::new()` boot — NO live /
    ///   debug tier, the smallest engine-less shell.
    /// - **debug client** (`--debug[=name]`): bootstrap the parked interpreter from
    ///   the EMBEDDED source (`bootstrap_from_bytes`, P3.1 — no fs needed) so a
    ///   flipped fn / breakpoint runs interpreted over the shared world, and bake
    ///   the debug NAME the client announces to the server for relay addressing.
    ///   Falls back to `Stores::new()` if the embedded bootstrap fails.
    fn emit_wasm_start(&self, w: &mut dyn Write) -> std::io::Result<()> {
        let (prelude, args) = self.entry_call_extra_args();
        Self::emit_asyncify_region(w)?;
        Self::emit_page_env_region(w)?;
        if self.emit_live {
            let name = self.debug_name.as_deref().unwrap_or("");
            let src = self.program_src.as_deref().unwrap_or("");
            write!(w, "static LOFT_LIVE_FNS: &[&str] = &[")?;
            for n in &self.live_fns {
                write!(w, "{n:?}, ")?;
            }
            writeln!(w, "];")?;
            writeln!(w, "static LOFT_DEBUG_NAME: &str = {name:?};")?;
            writeln!(w, "static LOFT_SRC: &str = {src:?};")?;
            write!(
                w,
                "\n#[unsafe(no_mangle)]\npub extern \"C\" fn loft_start() {{\n    \
                 loft::codegen_runtime::install_browser_panic_hook();\n    \
                 let _ = LOFT_DEBUG_NAME;\n    \
                 let cell = std::cell::UnsafeCell::new(\n        \
                 loft::live_dispatch::bootstrap_from_bytes(LOFT_LIVE_FNS, LOFT_SRC)\n            \
                 .unwrap_or_else(|e| {{ eprintln!(\"loft-debug: {{e}}\"); Stores::new() }}));\n    \
                 if loft::live_dispatch::live_enabled() {{ loft::live_dispatch::flip_all_dispatch_debug(); }} else {{ init(&cell); }}\n    \
                 {prelude}    n_main(&cell{args});\n    \
                 loft::live_dispatch::wasm_host_log(&format!(\"loft-debug: dispatched {{}} interp call(s) over the shared store\\n\", loft::live_dispatch::dispatch_count()));\n}}\n\
                 \n#[unsafe(no_mangle)]\npub extern \"C\" fn loft_debug_selftest() -> i32 {{\n    \
                 let r = loft::live_dispatch::wasm_debug_selftest();\n    \
                 loft::live_dispatch::wasm_host_log(&r);\n    \
                 loft::live_dispatch::wasm_host_log(\"\\n\");\n    \
                 i32::from(r == \"PAUSE n=40 STEP m=42 DONE=true\")\n}}\n\
                 \n// @PLN98 P3.4 — the interactive browser debug CLIENT: `loft_debug_start`\n\
                 // parses the embedded program into an interpreter session; the JS driver\n\
                 // then calls `loft_debug_pump` per frame to apply relayed `D!:` control\n\
                 // frames (host_input) and emit `D:` replies (host output).\n\
                 #[unsafe(no_mangle)]\npub extern \"C\" fn loft_debug_start() -> i32 {{\n    \
                 i32::from(loft::wasm_debug::start(LOFT_SRC))\n}}\n\
                 \n#[unsafe(no_mangle)]\npub extern \"C\" fn loft_debug_pump() {{\n    \
                 loft::wasm_debug::pump();\n}}\n"
            )
        } else {
            writeln!(
                w,
                "\n#[unsafe(no_mangle)]\npub extern \"C\" fn loft_start() {{\n    loft::codegen_runtime::install_browser_panic_hook();\n    let cell = std::cell::UnsafeCell::new(Stores::new());\n    init(&cell);\n{prelude}    n_main(&cell{args});\n}}"
            )
        }
    }

    fn emit_native_main(&self, w: &mut dyn Write) -> std::io::Result<()> {
        let (prelude, args) = self.entry_call_extra_args();
        // #255 / @PLN9: bake the parse-time `#cwd` path-mode default.
        writeln!(
            w,
            "const LOFT_PROGRAM_RELATIVE: bool = {};",
            self.stores.program_relative
        )?;
        // The program's own `.loft` path, baked in so the compiled binary can find its
        // log config in the same places the interpreter does (`.loft/log.conf`, then
        // `log.conf`, beside the program).  Without a logger the generated `n_log_*`
        // bodies have nowhere to write, which is how `--native` silently dropped every
        // `log_info` / `log_warn` / `log_error` / `log_fatal`.
        let main_file = {
            let d = self.data.def_nr("n_main");
            if d == u32::MAX {
                String::new()
            } else {
                self.data.def(d).position().file.clone()
            }
        };
        writeln!(w, "static LOFT_MAIN_FILE: &str = {main_file:?};")?;
        if self.emit_live {
            write!(w, "static LOFT_LIVE_FNS: &[&str] = &[")?;
            for n in &self.live_fns {
                write!(w, "{n:?}, ")?;
            }
            writeln!(w, "];")?;
            // @PLN98 P3.1 — the program's own source, embedded so the live boot can
            // bootstrap the parked interpreter from BYTES (no `LOFT_LIVE_SRC` file)
            // when the fs source is absent — the browser/wasm delivery.  `{:?}`
            // emits a valid escaped Rust string literal.
            match &self.program_src {
                Some(src) => writeln!(w, "static LOFT_SRC: Option<&str> = Some({src:?});")?,
                None => writeln!(w, "static LOFT_SRC: Option<&str> = None;")?,
            }
            // Emits `fn main`: arm the timeout watchdog + fail-fast (halt-at-op like the
            // interpreter, #333), then run init + n_main on a large-stack thread (@PLN28:
            // deep recursion trips MAX_CALL_DEPTH cleanly instead of overflowing the
            // ~8 MiB OS main-thread stack), then the optional native leak check.
            write!(
                w,
                "\nfn main() {{\n    loft::timeout::arm(loft::timeout::env_timeout_secs(), loft::timeout::env_grace_secs());\n    loft::database::NATIVE_FAIL_FAST.store(true, std::sync::atomic::Ordering::Relaxed);\n    let __run = || {{\n    let cell = std::cell::UnsafeCell::new(loft::live_dispatch::boot_stores(LOFT_LIVE_FNS, LOFT_SRC));\n    {{ let stores: &mut Stores = unsafe {{ &mut *cell.get() }}; stores.user_args = std::env::args().skip(1).collect(); stores.source_dir = Stores::source_dir_native(); stores.program_relative = LOFT_PROGRAM_RELATIVE; if let Ok(m) = std::env::var(\"LOFT_PATHS\") {{ stores.program_relative = m.eq_ignore_ascii_case(\"program\"); }} }}\n    {{ let stores: &mut Stores = unsafe {{ &mut *cell.get() }}; stores.logger = Some(std::sync::Arc::new(std::sync::Mutex::new(loft::logger::Logger::from_config_file(&loft::logger::Logger::resolve_config_path(std::env::var(\"LOFT_LOG_CONF\").ok().as_deref(), LOFT_MAIN_FILE), LOFT_MAIN_FILE)))); }}\n    if !loft::live_dispatch::live_enabled() {{ init(&cell); }}\n{prelude}    n_main(&cell{args});\n    {{ let stores: &Stores = unsafe {{ &*cell.get() }}; if stores.run_failed() {{ std::process::exit(1); }} }}\n"
            )?;
            writeln!(w, "    if !loft::live_dispatch::live_enabled() {{")?;
            w.write_all(NATIVE_LEAK_CHECK_TAIL.as_bytes())?;
            w.write_all(NATIVE_STRICT_STORE_TAIL.as_bytes())?;
            writeln!(w, "    }}")?;
            // @PLN103 P3 — the store-timeline summary (no-op unless LOFT_STORES=timeline);
            // runs on the native `__run` thread where the alloc/free events were recorded.
            writeln!(
                w,
                "    #[cfg(not(target_arch = \"wasm32\"))] if std::env::var(\"LOFT_STORES\").as_deref() == Ok(\"timeline\") {{ let stores: &Stores = unsafe {{ &*cell.get() }}; loft::database::timeline_summary(stores.collect_store_leaks().len()); }}"
            )?;
            writeln!(
                w,
                "    }};\n    \
                 // @PLN98 — the large-stack thread (@PLN28) is a NATIVE affordance;\n    \
                 // wasm32 (wasip2 / browser) has no thread support, and Android must run\n    \
                 // on its `android_main` thread (the ALooper owner — a spawned thread\n    \
                 // would panic in the graphics event pump), so both run `__run` inline.\n    \
                 #[cfg(all(not(target_arch = \"wasm32\"), not(target_os = \"android\")))]\n    \
                 if std::thread::Builder::new().stack_size(loft::codegen_runtime::NATIVE_MAIN_STACK).spawn(__run).expect(\"failed to spawn main-stack thread\").join().is_err() {{ std::process::exit(1); }}\n    \
                 // wasm32 has no thread to join, so the halt cannot be observed at a\n    \
                 // join point.  A panic HOOK runs before the abort, which makes the exit\n    \
                 // code reachable even where panics abort rather than unwind — without\n    \
                 // this a halting program exits 134 (SIGABRT) here, 101 natively and 1 on\n    \
                 // the interpreter: three backends, three answers, for every failed\n    \
                 // assertion.  The previous hook still runs, so the message is unchanged.\n    \
                 #[cfg(target_arch = \"wasm32\")]\n    \
                 {{ let prev = std::panic::take_hook(); std::panic::set_hook(Box::new(move |info| {{ prev(info); std::process::exit(1); }})); }}\n    \
                 #[cfg(any(target_arch = \"wasm32\", target_os = \"android\"))]\n    \
                 __run();"
            )?;
            writeln!(w, "}}")
        } else {
            // @PLN98 P2 — `--lean`: identical `main` MINUS the live/debug tier.
            // No `LOFT_LIVE_FNS` table; the world is a plain `Stores::new()`
            // (never `boot_stores`); `init` and the leak check run
            // UNCONDITIONALLY (no `live_enabled()` gate).  The emitted Rust
            // references no `live_dispatch` symbol at all.
            write!(
                w,
                "\nfn main() {{\n    loft::timeout::arm(loft::timeout::env_timeout_secs(), loft::timeout::env_grace_secs());\n    loft::database::NATIVE_FAIL_FAST.store(true, std::sync::atomic::Ordering::Relaxed);\n    let __run = || {{\n    let cell = std::cell::UnsafeCell::new(Stores::new());\n    {{ let stores: &mut Stores = unsafe {{ &mut *cell.get() }}; stores.user_args = std::env::args().skip(1).collect(); stores.source_dir = Stores::source_dir_native(); stores.program_relative = LOFT_PROGRAM_RELATIVE; if let Ok(m) = std::env::var(\"LOFT_PATHS\") {{ stores.program_relative = m.eq_ignore_ascii_case(\"program\"); }} }}\n    {{ let stores: &mut Stores = unsafe {{ &mut *cell.get() }}; stores.logger = Some(std::sync::Arc::new(std::sync::Mutex::new(loft::logger::Logger::from_config_file(&loft::logger::Logger::resolve_config_path(std::env::var(\"LOFT_LOG_CONF\").ok().as_deref(), LOFT_MAIN_FILE), LOFT_MAIN_FILE)))); }}\n    init(&cell);\n{prelude}    n_main(&cell{args});\n    {{ let stores: &Stores = unsafe {{ &*cell.get() }}; if stores.run_failed() {{ std::process::exit(1); }} }}\n"
            )?;
            w.write_all(NATIVE_LEAK_CHECK_TAIL.as_bytes())?;
            w.write_all(NATIVE_STRICT_STORE_TAIL.as_bytes())?;
            writeln!(
                w,
                "    #[cfg(not(target_arch = \"wasm32\"))] if std::env::var(\"LOFT_STORES\").as_deref() == Ok(\"timeline\") {{ let stores: &Stores = unsafe {{ &*cell.get() }}; loft::database::timeline_summary(stores.collect_store_leaks().len()); }}"
            )?;
            writeln!(
                w,
                "    }};\n    \
                 // @PLN98 — the large-stack thread (@PLN28) is a NATIVE affordance;\n    \
                 // wasm32 (wasip2 / browser) has no thread support, and Android must run\n    \
                 // on its `android_main` thread (the ALooper owner — a spawned thread\n    \
                 // would panic in the graphics event pump), so both run `__run` inline.\n    \
                 #[cfg(all(not(target_arch = \"wasm32\"), not(target_os = \"android\")))]\n    \
                 if std::thread::Builder::new().stack_size(loft::codegen_runtime::NATIVE_MAIN_STACK).spawn(__run).expect(\"failed to spawn main-stack thread\").join().is_err() {{ std::process::exit(1); }}\n    \
                 // wasm32 has no thread to join, so the halt cannot be observed at a\n    \
                 // join point.  A panic HOOK runs before the abort, which makes the exit\n    \
                 // code reachable even where panics abort rather than unwind — without\n    \
                 // this a halting program exits 134 (SIGABRT) here, 101 natively and 1 on\n    \
                 // the interpreter: three backends, three answers, for every failed\n    \
                 // assertion.  The previous hook still runs, so the message is unchanged.\n    \
                 #[cfg(target_arch = \"wasm32\")]\n    \
                 {{ let prev = std::panic::take_hook(); std::panic::set_hook(Box::new(move |info| {{ prev(info); std::process::exit(1); }})); }}\n    \
                 #[cfg(any(target_arch = \"wasm32\", target_os = \"android\"))]\n    \
                 __run();"
            )?;
            writeln!(w, "}}")
        }
    }

    /// Use this to emit only the `init` body that registers all types.
    /// Sorting by `known_type` ensures the runtime recreates type IDs in the same order
    /// as the compile-time database, keeping field indices consistent.
    fn output_init(&mut self, w: &mut dyn Write, from: u32, till: u32) -> std::io::Result<()> {
        // Base types are pre-registered by `Stores::new()` with fixed indices
        // 0..=6 (integer, long, single, float, boolean, text, character — see
        // `src/database/mod.rs:Stores::new`).  Subsequent struct / vector /
        // hash / index fields reference these by `known_type`.  The emitter
        // binds each pre-registered id into a `t{N}` variable so field
        // references use the same `t{N}` form as types created below — the
        // `known_type → runtime id` identity is made explicit via scope.
        for n in 0..=6u16 {
            writeln!(w, "    let t{n}: u16 = {n};")?;
        }
        let _ = writeln!(
            w,
            "    let _ = (t0, t1, t2, t3, t4, t5, t6); // suppress unused-let warnings for unreferenced base types"
        );
        let mut type_defs: Vec<(u16, u32)> = Vec::new();
        for dnr in from..till {
            self.start_fn(dnr);
            let def = self.data.def(dnr);
            let type_id = def.known_type();
            let is_enum_value_with_attrs =
                def.def_type() == DefType::EnumValue && !def.attributes().is_empty();
            if type_id != u16::MAX
                && (matches!(def.def_type(), DefType::Struct)
                    || def.def_type() == DefType::Enum
                    || def.def_type() == DefType::Vector
                    || is_enum_value_with_attrs)
            {
                type_defs.push((type_id, dnr));
            }
        }
        type_defs.sort_by_key(|(type_id, _)| *type_id);

        // Collect bare Byte/Short/Int types that were registered by
        // `database.byte` / `.short` / `.int` during type-field lowering
        // (e.g. narrow integer fields with `size(N)` annotations).  These
        // have no corresponding loft definition, so `output_init` would
        // otherwise skip them entirely, leaving a GAP in the runtime type
        // id sequence and shifting every type numbered after it.
        //
        // Enum values without attributes (plain tag variants) are also not
        // in `type_defs` — they don't get their own runtime type record,
        // but `Stores::enumerate` still advances the counter when the
        // parent enum is registered, so the plain-tag variants themselves
        // don't consume a slot.  Only Parts::{Byte, Short, Int} produce
        // standalone runtime types that need to be re-created here.
        let def_type_id_set: HashSet<u16> = type_defs.iter().map(|&(tid, _)| tid).collect();
        // @P296 — keyed-collection types reachable only as a struct/enum FIELD
        // are created inline during that container's field emission (so they
        // must NOT also be emitted in the bare_io stream — see the
        // Sorted/Hash/Index arm below).  But a keyed-collection LOCAL var
        // (`s: sorted<Item[k]> = []`) mints a keyed type via
        // `gen_set_first_keyed_null` that NO struct field references, so it
        // never gets created inline → its runtime type id is missing and
        // `content(tp)` returns u16::MAX → `set_default_value` panics on
        // `--native`.  Collect the field-referenced keyed type ids here so
        // the bare_io collection below can emit the local-only ones.
        //
        // loft#739 — "created inline by the container" holds only when the mint
        // really did FOLLOW the container. `fill_all` pre-registers the keyed
        // type of a LIBRARY-imported content struct while the library's own
        // definitions are still being filled, which is BEFORE the user struct
        // that carries the field is registered — so the keyed id PRECEDES its
        // container's. Leaving it out of the bare stream then creates it inline
        // at the container's position instead of its own, and since a keyed
        // type is created once, every runtime id from that point on is one
        // lower than the compile-time id baked into the emitted ops. The
        // observed symptoms were a `f#read as u16` silently returning null (its
        // `db_tp` const resolved to a struct) and a keyed lookup aborting with
        // `find called on non-collection type` naming an unrelated type.
        //
        // So decide per keyed type, using the LOWEST-id container that
        // references it — the one whose field emission would create it — and
        // exclude it only when its own id comes after that container.
        let field_keyed: HashSet<u16> = {
            let mut set = HashSet::new();
            let mut decided: HashSet<u16> = HashSet::new();
            for (idx, tp) in self.stores.types.iter().enumerate() {
                let container = idx as u16;
                if let crate::database::Parts::Struct(fields)
                | crate::database::Parts::EnumValue(_, fields) = &tp.parts
                {
                    for f in fields {
                        if matches!(
                            self.stores.types[f.content as usize].parts,
                            crate::database::Parts::Sorted(_, _)
                                | crate::database::Parts::Hash(_, _)
                                | crate::database::Parts::Index(_, _, _)
                        ) && decided.insert(f.content)
                            && f.content > container
                        {
                            set.insert(f.content);
                        }
                    }
                }
            }
            set
        };
        let mut bare_io: Vec<(u16, BareIo)> = Vec::new();
        for (idx, tp) in self.stores.types.iter().enumerate() {
            let tid = idx as u16;
            if def_type_id_set.contains(&tid) {
                continue;
            }
            match &tp.parts {
                crate::database::Parts::Byte(min, nullable) => {
                    bare_io.push((tid, BareIo::Byte(*min, *nullable)));
                }
                crate::database::Parts::Short(min, nullable) => {
                    bare_io.push((tid, BareIo::Short(*min, *nullable)));
                }
                crate::database::Parts::ShortRaw(min, nullable) => {
                    bare_io.push((tid, BareIo::ShortRaw(*min, *nullable)));
                }
                crate::database::Parts::Int(min, nullable) => {
                    bare_io.push((tid, BareIo::Int(*min, *nullable)));
                }
                crate::database::Parts::Vector(c) => {
                    bare_io.push((tid, BareIo::Vector(*c)));
                }
                // loft#1148 — `Array` is `Vector` AFTER the linked-group conversion:
                // `finish_type` rewrites a collection whose content struct is shared by
                // another collection (`linked`) and renames it `array<…>`.  It is the same
                // runtime type built the same way, so it is emitted as what it was BUILT as
                // and the generated `init()` re-derives the conversion exactly as the parse
                // did.
                //
                // Without this arm a converted type fell to `_ => {}` and got no `t{N}`
                // binding at all, while a type that REFERENCES it is still emitted — so
                // `init()` contained `let t85 = db.vector(t82);` with `t82` never bound and
                // `--native` handed the user `error[E0425]: cannot find value t82` for the
                // WHOLE program.  It takes two collections over ONE element struct to make
                // that struct linked, which is why distinct element structs are clean.
                crate::database::Parts::Array(c) => {
                    bare_io.push((tid, BareIo::Vector(*c)));
                }
                // `Ordered` is the same conversion applied to `Sorted`.  It was added for
                // symmetry with no failing case behind it, and the comment here asked
                // whoever saw it fire to record the shape.  **It fires**, and the shape is:
                //
                //     struct Box { data: vector<S?>, look: sorted<S[k]> }
                //
                // The `?` is what was missing from the shapes tried first.  A NULLABLE
                // element makes the sibling view's element type the synth `__nullable<S>`,
                // and the linked-group promotion then rewrites the field-referenced
                // `Sorted` to `Ordered` — so a converted keyed type reaches this stream
                // where a dense one never did.
                //
                // It also arrived carrying a defect, which is the argument for having kept
                // the arm: rendered through the short `bare_field_name`, the key list came
                // out `ordered<__nullable<W>[]>` — a name nothing else uses, so the call
                // MINTED a second collection type and shifted every runtime id after it
                // (loft#739's drift, caught by `verify_schema_ids`).  The cure was in
                // `bare_field_name`, not here; this arm was right all along and simply had
                // no reaching program until a nullable element supplied one.
                crate::database::Parts::Ordered(c, keys) => {
                    bare_io.push((tid, BareIo::Sorted(*c, keys.clone())));
                }
                // Sorted / Hash / Index that are a struct/enum FIELD are
                // created INLINE during that container's field emission
                // (via `emit_field` → `db.sorted / hash / index`), so this
                // stream would emit them a second time.  But a keyed type
                // minted only for a LOCAL var (@P296) is referenced by no
                // field, so it would otherwise leave a GAP in the runtime
                // type-id sequence (→ `content(tp)` u16::MAX → panic).  Emit
                // exactly those local-only keyed types here, at their tid
                // position; the bare_io arms below already know how.
                //
                // ⚠ **What makes a second emission safe is NAME AGREEMENT, not this
                // exclusion**, which is measured rather than argued.  Every `db.*`
                // collection constructor is interned by the type NAME it builds, so a
                // repeat call returns the existing id and the FIRST position wins; only
                // `db.index` has a side effect (the `#left_N` / `#right_N` / `#color_N`
                // triple it appends to the element), and it dedups early for exactly
                // that reason.  `Radix` / `Trie` are absent from `field_keyed` and so
                // are ALWAYS emitted twice for a field-referenced type — measured
                // harmless on both backends once the two call sites spell the key list
                // the same way.  They did not: the bare stream renders key numbers
                // through `bare_field_name`, which read the element's own field list
                // and answered `?` for a synth `__nullable<S>`, minting a SECOND
                // collection type under a name nothing else uses and shifting every
                // runtime id after it (loft#739's drift).  That is the invariant to
                // hold when touching this: the two spellings of a collection's key list
                // must agree, whatever this set contains.
                crate::database::Parts::Sorted(c, keys) if !field_keyed.contains(&tid) => {
                    bare_io.push((tid, BareIo::Sorted(*c, keys.clone())));
                }
                crate::database::Parts::Hash(c, keys) if !field_keyed.contains(&tid) => {
                    bare_io.push((tid, BareIo::Hash(*c, keys.clone())));
                }
                // @PLN48 — a local-only Radix (`spatial<T[…]>`) minted for a var,
                // referenced by no field: emit it here so it does not leave a gap in
                // the runtime type-id sequence (else `content(tp)` reads u16::MAX and
                // `record_new` panics), exactly as the local-only Hash arm above.
                crate::database::Parts::Trie(c, key) if !field_keyed.contains(&tid) => {
                    bare_io.push((tid, BareIo::Trie(*c, *key)));
                }
                crate::database::Parts::Radix(c, keys) if !field_keyed.contains(&tid) => {
                    bare_io.push((tid, BareIo::Radix(*c, keys.clone())));
                }
                crate::database::Parts::Index(c, keys, _) if !field_keyed.contains(&tid) => {
                    bare_io.push((tid, BareIo::Index(*c, keys.clone())));
                }
                crate::database::Parts::Sorted(_, _)
                | crate::database::Parts::Hash(_, _)
                | crate::database::Parts::Radix(_, _)
                | crate::database::Parts::Trie(_, _)
                | crate::database::Parts::Index(_, _, _) => {}
                _ => {}
            }
        }
        bare_io.sort_by_key(|&(tid, _)| tid);

        // Build a map from known_type → dnr for dependency resolution.
        let type_id_to_dnr: HashMap<u16, u32> =
            type_defs.iter().map(|&(tid, dnr)| (tid, dnr)).collect();

        // For each struct / enum-value / enum, collect the known_type ids that
        // its emission will *reference* as `t{N}` let-bindings — so the
        // topological walk emits them first.  Previously these were raw u16
        // literals and forward references worked; with the Category D let-
        // binding scheme, every referenced id must already be in scope.
        //
        // Struct / EnumValue: content-type of each sorted / hash / index /
        // vector field is a dep.  Enum: each typed variant (EnumValue with
        // attributes) is a dep, since `db.value(enum, variant_name, t{N})`
        // must find the variant's `t{N}` binding in scope.
        let mut deps: HashMap<u16, Vec<u16>> = HashMap::new();
        for &(type_id, dnr) in &type_defs {
            let def = self.data.def(dnr);
            let is_container = matches!(def.def_type(), DefType::Struct)
                || (def.def_type() == DefType::EnumValue && !def.attributes().is_empty());
            let is_enum = def.def_type() == DefType::Enum;
            if !is_container && !is_enum {
                continue;
            }
            let mut d: Vec<u16> = Vec::new();
            if is_container {
                for a in &def.attributes().to_vec() {
                    let c_nr = match &a.typedef {
                        Type::Sorted(c_nr, _, _)
                        | Type::Hash(c_nr, _, _)
                        | Type::Index(c_nr, _, _) => {
                            // Guard matches the Vector convention: skip unresolved (u32::MAX) content types.
                            (*c_nr != u32::MAX).then_some(*c_nr)
                        }
                        Type::Vector(c_type, _) => {
                            let n = self.data.type_def_nr(c_type);
                            (n != u32::MAX).then_some(n)
                        }
                        _ => None,
                    };
                    if let Some(c_nr) = c_nr {
                        let c_tp = self.data.def(c_nr).known_type();
                        if c_tp != u16::MAX && type_id_to_dnr.contains_key(&c_tp) {
                            d.push(c_tp);
                        }
                    }
                }
            } else {
                // is_enum: typed variants referenced by `db.value(enum, name, t{N})`.
                for a in &def.attributes().to_vec() {
                    if matches!(a.typedef, Type::Enum(_, true, _)) {
                        // Resolve the EnumValue def_nr whose parent matches.
                        let v_dnr = (0..self.data.definitions()).find(|&v| {
                            let vd = self.data.def(v);
                            vd.def_type == DefType::EnumValue
                                && vd.parent == dnr
                                && vd.name == a.name
                        });
                        if let Some(v_dnr) = v_dnr {
                            let v_tp = self.data.def(v_dnr).known_type();
                            if v_tp != u16::MAX && type_id_to_dnr.contains_key(&v_tp) {
                                d.push(v_tp);
                            }
                        }
                    }
                }
            }
            if !d.is_empty() {
                deps.insert(type_id, d);
            }
        }

        // Two-phase emission: (1) create all types in known_type order so every
        // cross-reference in phase 2 is a backward reference to an already-bound
        // `t{N}`; (2) populate struct / enum-value fields and enum values once
        // every type id is in scope.  This resolves mutual-recursion cycles
        // (e.g. JsonValue enum with JArray variant holding vector<JsonValue>)
        // that broke the previous single-pass topological approach.
        let _ = deps; // no longer used; kept only for future re-introduction
        let _ = type_id_to_dnr;

        // Single-pass emission in strict known_type order.
        //
        // For struct / enum-value types we emit `db.structure` + fields
        // immediately so that any subsequent bare Sorted/Hash/Index
        // (registered inline via the struct field emission) gets its
        // runtime id assigned at the exact moment that matches the
        // compile-time `known_type`.  For enums we only emit
        // `db.enumerate` here; the `db.value(enum, variant_tid)` calls
        // move to Phase 2 so that mutual-recursion cycles (enum →
        // typed variant → enum) break cleanly.
        //
        // `deps` / `type_id_to_dnr` are retired — known_type order is
        // sufficient because parse-time `fill_database` guarantees each
        // type's content dependencies already have a `known_type` by
        // the time the type itself is registered.
        // Track which type_ids have been fully emitted (creation +
        // fields) so the recursive walk below breaks cycles.  A type's
        // `db.structure` / `db.enumerate` call is emitted BEFORE
        // recursion, so by the time a field references it the `t{N}`
        // binding is already in scope — resolving the mutual-recursion
        // case (JsonValue enum ↔ JArray variant with
        // `items: vector<JsonValue>`).
        let mut emitted: HashSet<u16> = HashSet::new();
        let type_id_to_dnr_local = type_id_to_dnr;
        // `bare_emitted[tid]` tracks which bare types have had their
        // `db.*` creation emitted, so `flush_bare_through` never emits
        // one twice.  Sized to the full type table — bare type ids are
        // a subset of `0..stores.types.len()`.
        let mut bare_emitted = vec![false; self.stores.types.len()];

        for &(type_id, dnr) in &type_defs {
            // Emit every bare type that precedes this struct/enum in id
            // order (limit = type_id; no bare type shares a definition
            // type's id, so this is exactly `tid < type_id`).
            self.flush_bare_through(w, &bare_io, &mut bare_emitted, type_id)?;
            self.emit_def_create_recurse_fields(
                w,
                type_id,
                dnr,
                &deps,
                &type_id_to_dnr_local,
                &mut emitted,
                &bare_io,
                &mut bare_emitted,
            )?;
        }
        // Flush any bare types that follow the last definition (or that a
        // field's per-content flush has not already created).
        self.flush_bare_through(w, &bare_io, &mut bare_emitted, u16::MAX)?;

        // Phase 2 — enum value add-backs (db.value) emitted after all
        // typed variant structs have been created, so the enum ↔
        // variant mutual-recursion cycle resolves without forward refs.
        for &(type_id, dnr) in &type_defs {
            if self.data.def(dnr).def_type() == DefType::Enum {
                self.emit_type_fields_mode(
                    w,
                    type_id,
                    dnr,
                    FieldPhase::EnumValues,
                    &bare_io,
                    &mut bare_emitted,
                )?;
            }
        }
        // The emission above REPLAYS the parse-time registration order rather
        // than reading ids from it, and every type id baked into the generated
        // ops assumes the replay lands each type at the same index. Nothing
        // used to check that, so a type created one position early or late
        // shifted every id after it and a `db_tp` quietly named a neighbouring
        // type — the unattributable null reads and wrong-type abort of
        // loft#739. Close the loop where the two orders can be compared.
        self.write_schema_id_check(w)?;
        Ok(())
    }

    /// Initiative 03 Phase 3b: emit Rust code that rebuilds every
    /// `pub CONST: vector<T> = [...]` constant inside `init()`.
    /// Mirrors `compile::build_const_vectors` — for each DefType::
    /// Constant definition with vector content and literal values,
    /// allocates a fresh store, writes the elements, and records
    /// the DbRef in `db.const_refs[d_nr]`.  The `#rust"…"` template
    /// for `OpConstRef` is `s.const_ref_at(@d_nr as usize)`; native
    /// codegen translates `s.const_ref_at(` → `stores.const_ref_at_runtime(`
    /// (`src/generation/calls.rs`), so the emitted user functions
    /// find these DbRefs at call time.
    /// @PLN133 S8 — the lazy driver is a reachability ROOT.
    ///
    /// Nothing in loft calls `lazy_fetch`; the RUNTIME calls it, on a miss. So a
    /// walk rooted at `main` never reaches it, and `--native` would emit no body
    /// for the function it then installs a pointer to. The failure mode is the
    /// quiet one — a program that compiles and simply has no driver — which is
    /// why the root is added here rather than left to whoever writes the entry
    /// list.
    fn roots_with_lazy_driver(&self, entries: &[u32]) -> Vec<u32> {
        let mut roots = entries.to_vec();
        // @PLN133 S9 — EVERY driver is a root, not the one. A program declares
        // one per element type, and a driver left out of the walk is the quiet
        // failure again: the program compiles, installs a pointer to a body that
        // was never emitted, and that collection alone has no driver.
        if let Ok(drivers) = self.data.lazy_fetch_drivers() {
            for (_, d_nr) in drivers {
                if !roots.contains(&d_nr) {
                    roots.push(d_nr);
                }
            }
        }
        roots
    }

    /// @PLN133 S8/S9 — hand the program's lazy drivers to the runtime.
    ///
    /// `OpGetRecord` lives in libloft and cannot see a function the generator
    /// wrote, so `init()` installs a pointer per driver, keyed by the ELEMENT
    /// TYPE it serves. Which drivers exist and what each serves is decided in
    /// `Data::lazy_fetch_drivers` — the same place the interpreter asks — so the
    /// two backends cannot disagree about which driver a miss reaches.
    ///
    /// A driver with the wrong shape emits nothing, and the runtime then reports
    /// "needs a loft driver" rather than calling a function whose parameters do
    /// not line up — a wrong VALUE, which is the class this channel exists to
    /// keep out.
    fn emit_lazy_fetch_registration(&self, w: &mut dyn Write, till: u32) -> std::io::Result<()> {
        let drivers = match self.data.lazy_fetch_drivers() {
            Ok(d) => d,
            // The refusal travels as data. The interpreter re-asks `Data` at
            // every miss and reports the reason it wrote; native cannot ask, so
            // without this it registered nothing and reported "needs a loft
            // driver" — the same program naming a different mistake depending on
            // which backend you ran.
            Err(why) => {
                writeln!(
                    w,
                    "    codegen_runtime::register_lazy_fetch_refusal({why:?});"
                )?;
                return Ok(());
            }
        };
        for (element, d_nr) in drivers {
            if d_nr < till && (self.reachable.is_empty() || self.reachable.contains(&d_nr)) {
                writeln!(
                    w,
                    "    codegen_runtime::register_lazy_fetch({:?}, {});",
                    element,
                    self.data.def(d_nr).name
                )?;
            }
        }
        Ok(())
    }

    fn emit_const_vectors(&self, w: &mut dyn Write, till: u32) -> std::io::Result<()> {
        // Short-circuit if nothing references const_refs (avoids
        // emitting an unused `db.const_refs.resize(...)` that'd
        // produce a dead-code warning under `-D warnings`).
        let have_any = (0..till).any(|d| {
            self.data.def(d).def_type() == DefType::Constant && self.data.def(d).const_ref.is_some()
        });
        if !have_any {
            return Ok(());
        }
        writeln!(
            w,
            "    // Initiative 03 Phase 3b — const_refs: mirror compile::build_const_vectors.",
        )?;
        writeln!(
            w,
            "    db.const_refs.resize({till}, loft::keys::DbRef::NULL);"
        )?;
        for d_nr in 0..till {
            let def = self.data.def(d_nr);
            if def.def_type() != DefType::Constant {
                continue;
            }
            if def.const_ref.is_none() {
                continue;
            }
            let crate::data::Type::Vector(elem_tp_box, _) = def.returned() else {
                continue;
            };
            let elem_tp = (**elem_tp_box).clone();
            // Refused at the declaration when it cannot be pre-built (`parse_constant`
            // asks the same question), so a failure here has already been reported.
            let Ok(values) = crate::compile::extract_literal_values_public(def.code(), self.data)
            else {
                continue;
            };
            let vec_struct_name = format!("main_vector<{}>", elem_tp.name(self.data));
            let vec_struct_dnr = self.data.def_nr(&vec_struct_name);
            if vec_struct_dnr == u32::MAX {
                continue;
            }
            let vec_tp = self.data.def(vec_struct_dnr).known_type();
            if vec_tp == u16::MAX {
                continue;
            }
            let size = self.stores.size(vec_tp);
            writeln!(w, "    {{ // const d_nr={d_nr}")?;
            writeln!(w, "        let cv = db.database({size}_u32);")?;
            writeln!(
                w,
                "        db.store_mut(&cv).set_u32_raw(cv.rec, 4, {vec_tp}_u32);"
            )?;
            writeln!(w, "        db.set_default_value({vec_tp}, &cv);")?;
            writeln!(
                w,
                "        let cvr = loft::keys::DbRef {{ store_nr: cv.store_nr, rec: 1, pos: 8 }};"
            )?;
            for element in &values {
                writeln!(w, "        {{ let rec = db.record_new(&cvr, {vec_tp}, 0);")?;
                // Each field at ITS OWN offset within the element (loft#702) — the same
                // grouping `build_const_vectors` writes, from the same extractor.
                for (offset, val) in element {
                    let at = format!("rec.pos + {offset}");
                    // One arm per field kind, exhaustively — the interpreter's
                    // `build_const_vectors` writes the same bytes from the same list.
                    match val {
                        crate::compile::ConstField::Int(v) => {
                            writeln!(
                                w,
                                "            db.store_mut(&rec).set_int(rec.rec, {at}, {v}_i64);"
                            )?;
                        }
                        crate::compile::ConstField::Long(v) => {
                            writeln!(
                                w,
                                "            db.store_mut(&rec).set_long(rec.rec, {at}, {v}_i64);"
                            )?;
                        }
                        crate::compile::ConstField::Float(v) => {
                            writeln!(
                                w,
                                "            db.store_mut(&rec).set_float(rec.rec, {at}, {v}_f64);"
                            )?;
                        }
                        crate::compile::ConstField::Single(v) => {
                            writeln!(
                                w,
                                "            db.store_mut(&rec).set_single(rec.rec, {at}, {v}_f32);"
                            )?;
                        }
                        crate::compile::ConstField::Bool(v) => {
                            let byte = i32::from(*v);
                            writeln!(
                                w,
                                "            db.store_mut(&rec).set_byte(rec.rec, {at}, 0, {byte}_i32);"
                            )?;
                        }
                        crate::compile::ConstField::Char(v) => {
                            writeln!(
                                w,
                                "            db.store_mut(&rec).set_u32_raw(rec.rec, {at}, {v}_u32);"
                            )?;
                        }
                        crate::compile::ConstField::Text(v) => {
                            let esc = v.replace('\\', "\\\\").replace('"', "\\\"");
                            writeln!(
                                w,
                                "            {{ let store = db.store_mut(&rec); \
                                 let s_pos = store.set_str(\"{esc}\"); \
                                 store.set_u32_raw(rec.rec, {at}, s_pos); }}"
                            )?;
                        }
                    }
                }
                writeln!(w, "            db.record_finish(&cvr, &rec, {vec_tp}, 0);")?;
                writeln!(w, "        }}")?;
            }
            writeln!(w, "        db.allocations[cv.store_nr as usize].lock();")?;
            // Plan-57 Phase C: pin the const store (never freed) — see compile.rs.
            writeln!(
                w,
                "        db.allocations[cv.store_nr as usize].pinned = true;"
            )?;
            writeln!(w, "        db.const_refs[{d_nr}] = cvr;")?;
            writeln!(w, "    }}")?;
        }
        Ok(())
    }

    /// Recursive single-pass emission: create the type's `t{N}`
    /// binding first so any subsequent inline collection-field emission
    /// referencing it as `t{N}` finds the binding in scope.  Then
    /// recurse into content-type dependencies (from `deps`) to satisfy
    /// forward references like `JObject { fields: vector<JsonField> }`
    /// where `JsonField` has a higher `known_type` than `JObject`.
    /// Finally, emit fields in source order — inline collection creates
    /// dedup on name and land at the correct runtime id.
    /// Resolve a struct's field name by field_nr — needed for
    /// Sorted/Hash/Index key-string emission at bare-type level.
    /// The NAME of key field number `k` on collection content `c`, as the bare-stream
    /// `db.sorted` / `db.hash` / `db.spatial` / `db.trie` / `db.index` call spells it.
    ///
    /// Resolved against `Stores::key_owner`, the same field list the runtime constructors
    /// index — a synth `__nullable<S>` element keeps its keys in the `Some` payload, and the
    /// enum's own (empty) field list rendered every key as `?`.  That is not cosmetic: the key
    /// list is part of the type name, the name is the dedup key, so a `?` spelling MINTS a
    /// second collection type and every runtime id after it sits one above the compile-time id
    /// baked into the emitted ops (loft#739's drift, reported by `verify_schema_ids`).
    fn bare_field_name(&self, c: u16, k: u16) -> String {
        let owner = self.stores.key_owner(c);
        if let crate::database::Parts::Struct(ref fields)
        | crate::database::Parts::EnumValue(_, ref fields) =
            self.stores.types[owner as usize].parts
        {
            fields[k as usize].name.clone()
        } else {
            "?".to_string()
        }
    }

    /// Emit the `let t{tid} = db.xxx(…)` creation call for one bare
    /// (definition-less) runtime type, plus a `let _ = t{tid}` to
    /// suppress unused-binding warnings.  All `db.*` constructors are
    /// interned (return the existing id when the type already exists),
    /// so re-emitting a bare type is a harmless no-op — only the FIRST
    /// emission position determines its runtime id.
    fn write_bare_io(&self, w: &mut dyn Write, tid: u16, bio: &BareIo) -> std::io::Result<()> {
        match bio {
            BareIo::Byte(min, nullable) => {
                writeln!(w, "    let t{tid} = db.byte({min}, {nullable});")?;
            }
            BareIo::Short(min, nullable) => {
                writeln!(w, "    let t{tid} = db.short({min}, {nullable});")?;
            }
            BareIo::ShortRaw(min, nullable) => {
                writeln!(w, "    let t{tid} = db.short_raw({min}, {nullable});")?;
            }
            BareIo::Int(min, nullable) => {
                writeln!(w, "    let t{tid} = db.int({min}, {nullable});")?;
            }
            BareIo::Vector(c) => {
                let c_ref = type_id_ref(*c);
                writeln!(w, "    let t{tid} = db.vector({c_ref});")?;
            }
            BareIo::Sorted(c, keys) => {
                let c_ref = type_id_ref(*c);
                let keys_str = keys
                    .iter()
                    .map(|&(k, asc)| {
                        format!("(\"{}\".to_string(), {asc})", self.bare_field_name(*c, k))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(w, "    let t{tid} = db.sorted({c_ref}, &[{keys_str}]);")?;
            }
            BareIo::Hash(c, keys) => {
                let c_ref = type_id_ref(*c);
                let keys_str = keys
                    .iter()
                    .map(|&k| format!("\"{}\".to_string()", self.bare_field_name(*c, k)))
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(w, "    let t{tid} = db.hash({c_ref}, &[{keys_str}]);")?;
            }
            BareIo::Radix(c, keys) => {
                let c_ref = type_id_ref(*c);
                let keys_str = keys
                    .iter()
                    .map(|&k| format!("\"{}\".to_string()", self.bare_field_name(*c, k)))
                    .collect::<Vec<_>>()
                    .join(", ");
                // `db.spatial` is the surface constructor for the shared Radix kind.
                writeln!(w, "    let t{tid} = db.spatial({c_ref}, &[{keys_str}]);")?;
            }
            BareIo::Trie(c, key) => {
                let c_ref = type_id_ref(*c);
                let key_str = self.bare_field_name(*c, *key);
                writeln!(w, "    let t{tid} = db.trie({c_ref}, \"{key_str}\");")?;
            }
            BareIo::Index(c, keys) => {
                let c_ref = type_id_ref(*c);
                let keys_str = keys
                    .iter()
                    .map(|&(k, asc)| {
                        format!("(\"{}\".to_string(), {asc})", self.bare_field_name(*c, k))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(w, "    let t{tid} = db.index({c_ref}, &[{keys_str}]);")?;
            }
        }
        writeln!(w, "    let _ = t{tid}; // may be unused")?;
        Ok(())
    }

    /// Emit every not-yet-emitted bare type with `tid <= limit`, in
    /// ascending id order, marking each in `bare_emitted`.  Because
    /// `db.*` is interned, the runtime id a bare type receives is fixed
    /// at its FIRST creation, so this MUST be called in id order: the
    /// pre-struct flush (`limit = struct_id`) creates all bares that
    /// precede a struct, and the per-field flush (`limit = content_id`)
    /// creates a bare content type that a struct's field references but
    /// whose own id falls AFTER the struct (the @P353 inversion: an
    /// empty `vector<fn(…)>` wrapper struct is registered before its
    /// synthetic narrow-int element type).
    fn flush_bare_through(
        &self,
        w: &mut dyn Write,
        bare_io: &[(u16, BareIo)],
        bare_emitted: &mut [bool],
        limit: u16,
    ) -> std::io::Result<()> {
        for (tid, bio) in bare_io {
            if *tid > limit {
                break;
            }
            if !bare_emitted[*tid as usize] {
                self.write_bare_io(w, *tid, bio)?;
                bare_emitted[*tid as usize] = true;
            }
        }
        Ok(())
    }

    #[allow(clippy::only_used_in_recursion, clippy::too_many_arguments)]
    fn emit_def_create_recurse_fields(
        &mut self,
        w: &mut dyn Write,
        type_id: u16,
        dnr: u32,
        deps: &HashMap<u16, Vec<u16>>,
        type_id_to_dnr: &HashMap<u16, u32>,
        emitted: &mut HashSet<u16>,
        bare_io: &[(u16, BareIo)],
        bare_emitted: &mut [bool],
    ) -> std::io::Result<()> {
        if !emitted.insert(type_id) {
            return Ok(());
        }
        // Emit type-creation call first so the `t{type_id}` binding is
        // available for any recursive emission that reads it as a
        // content type below.
        self.emit_type_creation(w, type_id, dnr)?;
        if dnr == u32::MAX {
            return Ok(());
        }
        let def = self.data.def(dnr);
        if matches!(def.def_type(), DefType::Struct)
            || (def.def_type() == DefType::EnumValue && !def.attributes().is_empty())
        {
            // Walk fields in source order, mirroring parse-time
            // `fill_database` exactly.  For each collection field
            // (`vector / sorted / hash / index<X>`), recurse into the
            // content type X *inline* — so X's `db.structure` call (and
            // any type X's own fields trigger) land at the same
            // runtime index as parse time.  Then emit the field itself,
            // which triggers inline `db.vector / sorted / hash / index`
            // creation at the next runtime id — matching parse-time.
            let enum_value = if def.def_type() == DefType::EnumValue {
                let parent = self.data.def(def.parent);
                parent
                    .attributes
                    .iter()
                    .enumerate()
                    .find(|(_, a)| a.name == def.name())
                    .map_or(0, |(i, _)| i32::try_from(i).unwrap_or(0) + 1)
            } else {
                0
            };
            let s_var = format!("t{type_id}");
            if enum_value > 0
                && def.known_type() != u16::MAX
                && self.stores.position(def.known_type(), "enum") == 0
            {
                writeln!(w, "    let byte_enum = db.byte(0, false);")?;
                writeln!(w, "    db.field({s_var}, \"enum\", byte_enum);")?;
            }
            let attrs = def.attributes().to_vec();
            for a in &attrs {
                // Resolve field's content dep and recurse inline
                // before the field is emitted — parse-time
                // `fill_database` does the same via recursive content
                // resolution when a collection field first names a
                // forward-declared type.
                //
                // Peel the `?` first, exactly as `fill_database` does: an `Optional(τ)`
                // field lays out as `τ`, so the hoist it needs is `τ`'s.  Matching the
                // wrapper instead landed every arm below on `None`, and a nullable
                // forward-referenced field emitted `db.field(t_host, "f", t_content)`
                // ahead of `let t_content = …` — a Rust compile error in the generated
                // `init()`, i.e. the library simply failed to build (loft#797).
                let field_type = a.typedef.base();
                let dep_tp = match field_type {
                    // loft#1222 — every KEYED former belongs here, and `Trie` / `Radix` were
                    // missing.  Their field emitters build the collection inline exactly as
                    // Sorted / Hash / Index do (`db.trie(c_ref, key)`, `db.spatial(c_ref,
                    // keys)`), so they need the content type declared just as much; falling to
                    // `_ => None` meant no hoist, and a `trie<Tk[k]>` or `spatial<Pt[x,y]>`
                    // field whose element struct is declared LATER emitted `db.trie(t80, "k")`
                    // two lines before `let t80 = …`.  rustc rejected the generated `init()`
                    // with E0425 while the interpreter ran the same program — a backend
                    // divergence reported in a language the author never wrote.
                    //
                    // The set is the five store-backed keyed formers, which is what
                    // `emit_db_field`'s arms above already enumerate; keeping the two lists in
                    // step is the whole obligation here.
                    Type::Sorted(c_nr, _, _)
                    | Type::Hash(c_nr, _, _)
                    | Type::Index(c_nr, _, _)
                    | Type::Trie(c_nr, _, _)
                    | Type::Radix(c_nr, _, _) => (*c_nr != u32::MAX)
                        .then(|| self.data.def(*c_nr).known_type())
                        .filter(|t| *t != u16::MAX),
                    Type::Vector(c_type, _) => {
                        let n = self.data.type_def_nr(c_type);
                        (n != u32::MAX)
                            .then(|| self.data.def(n).known_type())
                            .filter(|t| *t != u16::MAX)
                    }
                    // Plan-06 phase 4d: tuple struct fields inline the
                    // synthetic `__tuple<…>` struct's bytes — emit its
                    // type-creation call before the parent's `db.field`
                    // so the forward reference (`db.field(t_parent, "v",
                    // t_synthetic_tuple)`) sees the synthetic binding
                    // already declared.
                    Type::Tuple(_) => {
                        let n = self.data.type_def_nr(field_type);
                        (n != u32::MAX)
                            .then(|| self.data.def(n).known_type())
                            .filter(|t| *t != u16::MAX)
                    }
                    // #313: a split fn-ref field (capturing closure
                    // assigned) registers `db.child_rec(t{closure})` —
                    // recurse into the closure-record struct first so
                    // its `t{N}` binding precedes the field emission,
                    // mirroring `fill_database`'s inline recursion.
                    Type::Function(_, _, _) => (a.assigned_lambda_d_nr != u32::MAX)
                        .then(|| self.data.def(a.assigned_lambda_d_nr).closure_record())
                        .filter(|cr| *cr != u32::MAX)
                        .map(|cr| self.data.def(cr).known_type())
                        .filter(|t| *t != u16::MAX),
                    // An embedded struct/enum-reference field (`inner: Cell`,
                    // empty deps) stores the content's bytes INLINE, so the host's
                    // `db.field` needs the content type already declared.  When the
                    // host is defined BEFORE the content — a forward or
                    // cross-package reference — the content's `db.structure` would
                    // otherwise land after the host's `db.field`, a use-before-
                    // declare that corrupts the field offset (the native sibling of
                    // the interp `fill_database` default-arm recursion, @P373).
                    // The `!emitted` guard below makes this a no-op unless the
                    // content really is still forward.  Non-empty deps = a 12-byte
                    // dbref (not inline) — no size dependency, so no hoist.
                    Type::Reference(c_nr, ref_deps) if ref_deps.is_empty() => (*c_nr != u32::MAX)
                        .then(|| self.data.def(*c_nr).known_type())
                        .filter(|t| *t != u16::MAX),
                    _ => None,
                };
                if let Some(dep_tp) = dep_tp
                    && !emitted.contains(&dep_tp)
                    && let Some(&dep_dnr) = type_id_to_dnr.get(&dep_tp)
                {
                    self.emit_def_create_recurse_fields(
                        w,
                        dep_tp,
                        dep_dnr,
                        deps,
                        type_id_to_dnr,
                        emitted,
                        bare_io,
                        bare_emitted,
                    )?;
                }
                let td_nr = self.data.type_def_nr(&a.typedef);
                let field_type_id = self.data.def(td_nr).known_type();
                let forced = self.data.forced_size(a.alias_d_nr);
                self.emit_field(
                    w,
                    &s_var,
                    type_id,
                    &a.name,
                    &a.typedef,
                    a.nullable,
                    field_type_id,
                    forced,
                    bare_io,
                    bare_emitted,
                    &a.value,
                )?;
            }
            // PLAN51 Cluster V-a — re-register the synthetic tuple's
            // LinkedFieldGroup at runtime.  The parser side propagates
            // it in `typedef.rs::fill_database` (line 567-570); without
            // this mirror call, the generated binary's `init()` rebuilds
            // the tuple Type with empty `field_groups`, `finish_type`
            // falls back to the simple alignment-descending packer, and
            // tuple field positions/size diverge from what the compile-
            // side IR was generated against.  Probes 29, 41, 44, 45,
            // 48, 50 in the PLAN51 probe suite cover the failure modes.
            if let Some(group) = self.data.def(dnr).tuple_group() {
                let idx_list = group
                    .field_indices
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(w, "    db.add_tuple_group(t{type_id}, &[{idx_list}]);")?;
            }
        }
        Ok(())
    }

    /// Phase 1 — emit just the type-creation call for `dnr` (no fields,
    /// Emit the `init()`-closing call that compares the schema the generated
    /// program just built against the compiler's own table, index by index.
    ///
    /// See [`Stores::verify_schema_ids`] for why this exists: the two orders
    /// are derived independently and a single misplaced type silently renames
    /// every id after it.  The names are emitted in compile-time index order,
    /// which is exactly the order `init()` is supposed to reproduce.
    fn write_schema_id_check(&self, w: &mut dyn Write) -> std::io::Result<()> {
        writeln!(w, "    db.verify_schema_ids(&[")?;
        for tp in &self.stores.types {
            writeln!(w, "        {:?},", tp.name)?;
        }
        writeln!(w, "    ]);")?;
        Ok(())
    }

    /// no enum values).  Captures the runtime id in a `let t{type_id}`
    /// binding so Phase 2 field/value emission can reference it.
    fn emit_type_creation(
        &mut self,
        w: &mut dyn Write,
        type_id: u16,
        dnr: u32,
    ) -> std::io::Result<()> {
        if dnr == u32::MAX {
            eprintln!(
                "codegen warning: skipping type_id={type_id} — definition number is unresolved (u32::MAX)"
            );
            return Ok(());
        }
        let def = self.data.def(dnr);
        // @P379 — emit the REGISTERED database name (library-qualified on a
        // cross-library bare-name collision, e.g. `moros_map::Chunk`), not
        // the bare def name, so the generated `db.structure(...)` matches the
        // interpreter's table and two same-named library structs don't
        // collide in generated code either.
        //
        // `{name:?}` writes the literal, rather than pasting the name between two typed
        // quotes: a registered name is not always an identifier.  A synthetic wrapper over
        // a KEYED collection carries its key list — `main_vector<hash<E,["k"]>>` — and the
        // quotes in it closed the Rust string early, so `vector<hash<E[k]>>` (with or
        // without a `?`) aborted `--native` with four rustc errors from one line of
        // generated code, while `--interpret` ran it.  Debug-formatting escapes them and
        // leaves every ordinary name byte-identical.
        let reg_name = &self.stores.types[type_id as usize].name;
        if matches!(def.def_type(), DefType::Struct) {
            writeln!(w, "    let t{type_id} = db.structure({reg_name:?}, 0);")?;
        } else if def.def_type() == DefType::EnumValue && !def.attributes().is_empty() {
            let parent_nr = def.parent;
            if parent_nr == u32::MAX {
                return Ok(());
            }
            let parent = self.data.def(parent_nr);
            let enum_value = parent
                .attributes
                .iter()
                .enumerate()
                .find(|(_, a)| a.name == def.name())
                .map_or(0, |(i, _)| i32::try_from(i).unwrap_or(0) + 1);
            writeln!(
                w,
                "    let t{type_id} = db.structure({reg_name:?}, {enum_value});"
            )?;
        } else if def.def_type() == DefType::Enum {
            writeln!(w, "    let t{type_id} = db.enumerate({:?});", def.name())?;
        } else if def.def_type() == DefType::Vector {
            // prefer the actual registered Parts::Vector
            // content from `stores.types[type_id]` — that's what
            // `fill_database` stored, including narrow-integer content
            // via `narrow_vector_content`.  Falling back to
            // `def.parent.known_type` resolves any Type::Integer to the
            // plain `integer` slot (0) and breaks narrow vectors.
            let content_known = if let crate::database::Parts::Vector(c) =
                self.stores.types[type_id as usize].parts
            {
                c
            } else if def.parent != u32::MAX {
                self.data.def(def.parent).known_type()
            } else if let Type::Vector(c_type, _) = def.returned() {
                let c_dnr = self.data.type_def_nr(c_type);
                if c_dnr == u32::MAX {
                    u16::MAX
                } else {
                    self.data.def(c_dnr).known_type()
                }
            } else {
                u16::MAX
            };
            if content_known != u16::MAX {
                let content_ref = type_id_ref(content_known);
                writeln!(w, "    let t{type_id} = db.vector({content_ref});")?;
                writeln!(w, "    let _ = t{type_id}; // may be unused")?;
            }
        }
        Ok(())
    }

    /// Populate struct / enum-value fields or enum values.  `mode` selects
    /// which kinds of fields to emit:
    ///   `FieldPhase::Simple`      — scalar / text / enum-typed fields only.
    ///   `FieldPhase::Collection`  — Vector / Sorted / Hash / Index fields
    ///                               (reference pre-created `t{N}` bare
    ///                               types emitted by Phase 1).
    ///   `FieldPhase::EnumValues`  — enum value add-backs (`db.value`).
    fn emit_type_fields_mode(
        &self,
        w: &mut dyn Write,
        type_id: u16,
        dnr: u32,
        mode: FieldPhase,
        bare_io: &[(u16, BareIo)],
        bare_emitted: &mut [bool],
    ) -> std::io::Result<()> {
        if dnr == u32::MAX {
            return Ok(());
        }
        let def = self.data.def(dnr);
        if matches!(def.def_type(), DefType::Struct)
            || (def.def_type() == DefType::EnumValue && !def.attributes().is_empty())
        {
            if matches!(
                mode,
                FieldPhase::Simple | FieldPhase::Collection | FieldPhase::AllFields
            ) {
                let enum_value = if def.def_type() == DefType::EnumValue {
                    let parent = self.data.def(def.parent);
                    parent
                        .attributes
                        .iter()
                        .enumerate()
                        .find(|(_, a)| a.name == def.name())
                        .map_or(0, |(i, _)| i32::try_from(i).unwrap_or(0) + 1)
                } else {
                    0
                };
                self.output_struct_fields_filtered(
                    w,
                    dnr,
                    enum_value,
                    type_id,
                    mode,
                    bare_io,
                    bare_emitted,
                )?;
            }
        } else if def.def_type() == DefType::Enum && matches!(mode, FieldPhase::EnumValues) {
            output_enum_values(w, dnr, self.data, type_id)?;
        }
        Ok(())
    }

    /// Use this to emit all function bodies for the given definition range.
    /// When `reachable` is Some, only functions in the set are emitted.
    fn output_functions(
        &mut self,
        w: &mut dyn Write,
        from: u32,
        till: u32,
        reachable: Option<&HashSet<u32>>,
    ) -> std::io::Result<()> {
        // @PLN11 G2/M2 — persistent program store (mirrors the interpreter's
        // byte_code_from): materialise the whole Data once and read each body
        // node from it; default off → IrBlock::Native.
        let program_store = if std::env::var_os("LOFT_CODEGEN_STORE").is_some() {
            let mut stores = Stores::new();
            let root = crate::ir_store::materialize_data(&mut stores, self.data);
            Some((stores, root))
        } else {
            None
        };
        for dnr in from..till {
            if !matches!(self.data.def(dnr).def_type(), DefType::Function) {
                continue;
            }
            if let Some(r) = reachable
                && !r.contains(&dnr)
            {
                continue;
            }
            self.output_function(w, dnr, program_store.as_ref())?;
        }
        Ok(())
    }

    /// The content type id `fill_database` recorded for `host_type_id`'s field
    /// `field_name`, if the host is a struct/enum-value with such a field.
    ///
    /// The compiler's own answer to "what type does this field hold", including
    /// any element narrowing it applied. Reading it beats re-deriving one from
    /// the loft `Type`, which cannot see a `forced_size` through
    /// `type_def_nr` (loft#742).
    fn recorded_field_content(&self, host_type_id: u16, field_name: &str) -> Option<u16> {
        let (crate::database::Parts::Struct(fields) | crate::database::Parts::EnumValue(_, fields)) =
            &self.stores.types.get(host_type_id as usize)?.parts
        else {
            return None;
        };
        fields
            .iter()
            .find(|f| f.name == field_name)
            .map(|f| f.content)
            .filter(|c| *c != u16::MAX)
    }

    /// Use this to emit a single struct field into the db-builder output.
    /// Dispatches on the field's `typedef` to produce the correct `db.*` call.
    /// `s_var` is the Rust variable holding the parent struct's runtime id
    /// (e.g. `t59` for `known_type=59`).
    #[allow(clippy::too_many_arguments)]
    /// Register one field in the generated `init()`, then — @PLN127 arc D —
    /// replay the nullability deposit beside it.
    ///
    /// `--native` rebuilds the schema by REPLAYING `init()`, so a fact the parser
    /// deposits and the generator does not emit is simply ABSENT there.
    /// Reflection answered `nullable=false` for every field on the native backend
    /// until this existed, and the parity probe is what caught it. It wraps the
    /// emitter rather than sitting in either caller because there are two call
    /// sites and the one that mattered was not the obvious one.
    #[allow(clippy::too_many_arguments)]
    fn emit_field(
        &self,
        w: &mut dyn Write,
        s_var: &str,
        host_type_id: u16,
        field_name: &str,
        typedef: &Type,
        nullable: bool,
        known_type: u16,
        forced_size: Option<u8>,
        bare_io: &[(u16, BareIo)],
        bare_emitted: &mut [bool],
        declared_default: &Value,
    ) -> std::io::Result<()> {
        self.emit_field_inner(
            w,
            s_var,
            host_type_id,
            field_name,
            typedef,
            nullable,
            known_type,
            forced_size,
            bare_io,
            bare_emitted,
        )?;
        if nullable || matches!(typedef, Type::Optional(_)) {
            writeln!(
                w,
                "    db.set_field_nullable({s_var}, \"{field_name}\", true);"
            )?;
        }
        // loft#876 — replay the field's DECLARED constant default.  The schema this
        // `init()` rebuilds is the one the JSON walker consults for "what is this field
        // when nobody said", so without this line a `text as Struct` cast answers the
        // type's zero on `--native` while the interpreter answers the default.  Folded
        // by the same function the parse-time deposit uses, so the two backends cannot
        // disagree about which defaults are constant.
        if let Some(c) = crate::typedef::fold_declared_default(declared_default) {
            let lit = match c {
                crate::keys::Content::Long(n) => format!("loft::keys::Content::Long({n}_i64)"),
                // `{:?}` on a float round-trips exactly (`1.5` → `1.5`); a declared
                // default is a literal, so there is no NaN / infinity to spell.
                crate::keys::Content::Float(v) => format!("loft::keys::Content::Float({v:?}_f64)"),
                crate::keys::Content::Single(v) => {
                    format!("loft::keys::Content::Single({v:?}_f32)")
                }
                crate::keys::Content::Str(s) => {
                    format!(
                        "loft::keys::Content::Str(loft::keys::Str::new({:?}))",
                        s.str()
                    )
                }
            };
            writeln!(
                w,
                "    db.set_field_default({s_var}, \"{field_name}\", {lit});"
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_field_inner(
        &self,
        w: &mut dyn Write,
        s_var: &str,
        host_type_id: u16,
        field_name: &str,
        typedef: &Type,
        nullable: bool,
        known_type: u16,
        forced_size: Option<u8>,
        bare_io: &[(u16, BareIo)],
        bare_emitted: &mut [bool],
    ) -> std::io::Result<()> {
        // @PLN25 — an `Optional(τ)` field carries its nullability in the wrapper;
        // peel it so the type-dispatch below sees the base type, and fold the
        // wrapper into `nullable` so a nullable NARROW int (`x: u8?` /
        // `integer limit(..)?`) takes the `db.byte`/`db.short`/`db.int` path (correct
        // 1/2/4-byte storage width) instead of falling through to an 8-byte type
        // ref. Without this the struct sized to 8 bytes on native (but 2 on interp),
        // so a `vector<struct>` appended elements at 8-byte stride while reads used
        // the 2-byte access stride → every element past index 0 read back null. Inert
        // gate-OFF (no `Optional` is ever constructed, so `.base()` is a no-op).
        let nullable = nullable || matches!(typedef, Type::Optional(_));
        let typedef = typedef.base();
        if let Type::Vector(c, _) = typedef {
            // loft#923 — peel the ELEMENT's `Optional` for the same reason line 3487
            // peels the field's: `Optional(τ)` shares τ's storage exactly, and the
            // compiler's own table holds `vector<τ?>` and `vector<τ>` as ONE type.
            // Unpeeled, a `vector<vector<integer>?>` field missed the nested-vector
            // arm below and fell through to the generic path, where
            // `type_def_nr(Type::Optional)` resolves to the generic `vector` def and
            // its `known_type` is whatever registered last — `db.vector(t7)`, an
            // element type the program never named. That MINTS a type, so every
            // runtime id from there on sat one above the compile-time id baked into
            // the emitted ops, and loft#739's guard reported the drift.
            // The element's own nullability, read BEFORE the peel below: a nullable
            // narrow element reserves a null code, which is both a possible WIDTH
            // change and a different Part (`byte<min,true>` / `short`), so the name
            // this site looks up has to carry it — `narrow_vector_content` registered
            // it that way.
            let elm_nullable = matches!(c.as_ref(), Type::Optional(_));
            let c: &Type = c.base();
            // when the element is a narrow `Type::Integer` — by its `size(N)`
            // annotation OR (loft#1036) by a `limit(lo, hi)` range that fits 1/2/4
            // bytes — look up the narrow content type-nr that `fill_database`
            // already registered via `narrow_vector_content`.  Without this,
            // `data.type_def_nr(c)` resolves any `Type::Integer` to
            // the plain `integer` def-nr → wide `vector<integer>`.
            // The wrapper's `main_vector<T>` struct field would end up
            // with 8-byte stride even though `fill_database` narrowed
            // the actual runtime Parts, corrupting reads/writes.
            if let Type::Integer(spec) = c
                && let Some(n) = spec.vector_narrow_width(elm_nullable)
            {
                // The NAME is the schema key, so it has to carry the same offset
                // `narrow_vector_content` registered — `part_min`, not the declared `min`
                // (they differ for a nullable signed narrow element).
                let elm_min = spec.part_min(n, elm_nullable);
                let name = match n {
                    1 => {
                        if elm_min == 0 && !elm_nullable {
                            "byte".to_string()
                        } else {
                            format!("byte<{elm_min},{elm_nullable}>")
                        }
                    }
                    // A nullable 2-byte element is the `+1` sentinel encoding
                    // (`Parts::Short`), the non-null one direct (`ShortRaw`) —
                    // mirroring `Data::narrow_vector_content`.
                    2 if elm_nullable => format!("short<{elm_min},true>"),
                    2 => format!("short_raw<{elm_min},false>"),
                    4 => format!("int<{elm_min},{elm_nullable}>"),
                    _ => String::new(),
                };
                if !name.is_empty() {
                    let narrow = self.stores.name(&name);
                    if narrow != u16::MAX {
                        // @P353: the narrow element type may have a higher
                        // runtime id than the wrapper struct (created as a
                        // side-effect of this field's registration), so flush
                        // its `db.*` creation before referencing it.
                        self.flush_bare_through(w, bare_io, bare_emitted, narrow)?;
                        let content_ref = type_id_ref(narrow);
                        emit_db_field(
                            w,
                            s_var,
                            field_name,
                            "vec",
                            &format!("db.vector({content_ref})"),
                        )?;
                        return Ok(());
                    }
                }
            }
            // P214: vector<fn-ref> elements route through narrow_vector_content
            // to a 4-byte int (Parts::Int with size=4) — match the parser's
            // `vector_of` path so the emitted `db.vector(narrow_int)`
            // matches the runtime narrow Parts.  Without this, the
            // `type_def_nr(Type::Function)` path returns `i32` whose
            // `known_type` is `u16::MAX` → emits `db.vector(u16::MAX)`
            // which panics in `Stores::field`'s parent-tracking when the
            // wrapper struct is registered.
            if matches!(c, Type::Function(_, _, _)) {
                let narrow = self.stores.name("int<0,false>");
                if narrow != u16::MAX {
                    // @P353: an empty `vector<fn(…)>` literal registers its
                    // wrapper struct before this synthetic narrow-int element
                    // type, so `int<0,false>`'s runtime id is HIGHER than the
                    // struct's — flush its creation before the `db.vector`.
                    self.flush_bare_through(w, bare_io, bare_emitted, narrow)?;
                    let content_ref = type_id_ref(narrow);
                    emit_db_field(
                        w,
                        s_var,
                        field_name,
                        "vec",
                        &format!("db.vector({content_ref})"),
                    )?;
                    return Ok(());
                }
            }
            // @PLAN52 cluster IV-Vec-nested-field-push secondary (2026-05-30):
            // for nested-vector content `vector<vector<X>>`, `c =
            // Type::Vector(inner_c)`.  `type_def_nr(Type::Vector)` returns the
            // GENERIC "vector" d_nr (per `data.rs:3186`); reading its
            // `known_type` returns garbage (last-assigned value, e.g.
            // FieldValue's id when the default lib registered `vector<FieldValue>`).
            // Look up the concrete nested vector type by name (parser's
            // database has already registered it).  Then emit a chained
            // `db.vector(db.vector(<inner>))` so the runtime database
            // registers the nested layers in the same order, regardless of
            // which other types happened to register `vector<X>` first.
            if matches!(c, Type::Vector(_, _)) {
                // loft#742 — prefer the content id `fill_database` RECORDED for
                // this field over anything re-derived from the loft `Type`.
                //
                // The chain below rebuilds the nesting from `Type`, and a
                // `Type::Integer` carries its narrowing in a `forced_size` that
                // `type_def_nr` throws away: both `vector<vector<integer>>` and
                // `vector<vector<integer(-32768, 32767)>>` came out as
                // `db.vector(db.vector(<plain integer>))`. That is the wrong
                // ELEMENT WIDTH, and it also mints `vector<vector<integer>>` —
                // a type the compiler holds at a different id, or not at all at
                // that point — so every runtime id after it shifted by one and
                // `verify_schema_ids` flagged it.
                //
                // The recorded content is already narrowed
                // (`vector<vector<short_raw<-32768,false>>>`) and already
                // registered, so referencing it is both the right width and the
                // right id. Same move as the sibling arms above and below,
                // which read a content id rather than rebuilding one.
                // Only when the content's `let t{N}` is ALREADY in scope — a
                // bare type that has been flushed, or a definition type with a
                // lower id (`type_defs` is sorted by id and emits each
                // creation before its fields, so a lower id is already bound).
                //
                // Deliberately no `flush_bare_through` here to make one
                // available. Flushing a bare VECTOR early emits
                // `db.vector(t73)` for an element whose own binding the walk
                // has not reached yet (E0425 in `186_nested_comprehension`),
                // and referencing an unflushed id has the same fault one level
                // up (E0425 in 11 scripts). A forward reference falls through
                // to the chain below, exactly as before — no worse than it was,
                // and the shapes this issue is about are backward references.
                let content = self.recorded_field_content(host_type_id, field_name);
                if let Some(content) = content
                    && (if bare_io.iter().any(|(tid, _)| *tid == content) {
                        bare_emitted[content as usize]
                    } else {
                        content < host_type_id
                    })
                {
                    emit_db_field(w, s_var, field_name, "vec", &type_id_ref(content))?;
                    return Ok(());
                }
                // Count nesting depth and find the innermost non-Vector type.
                // For `vector<vector<...vector<X>>>`, emit
                // `{ let _v0 = db.vector(<X_ref>); let _v1 = db.vector(_v0); ...
                //   db.vector(_vN) }`.  Each layer registers (or retrieves) at
                // runtime in order, so any prior `db.vector(other_inner)`
                // (e.g. `vector<FieldValue>` from a default-lib helper) no
                // longer shifts our nested vectors' slot ids.
                let mut depth = 1;
                let mut innermost: &Type = c;
                while let Type::Vector(next, _) = innermost {
                    innermost = next;
                    depth += 1;
                }
                let inner_def = self.data.type_def_nr(innermost);
                if inner_def != u32::MAX {
                    use std::fmt::Write as _;
                    let inner_kt = self.data.def(inner_def).known_type();
                    self.flush_bare_through(w, bare_io, bare_emitted, inner_kt)?;
                    let inner_type_ref = type_id_ref(inner_kt);
                    let mut expr = format!("{{ let _v0 = db.vector({inner_type_ref});");
                    for level in 1..depth {
                        let prev = level - 1;
                        write!(&mut expr, " let _v{level} = db.vector(_v{prev});").unwrap();
                    }
                    let last = depth - 1;
                    write!(&mut expr, " _v{last} }}").unwrap();
                    emit_db_field(w, s_var, field_name, "vec", &expr)?;
                    return Ok(());
                }
            }
            let c_def = self.data.type_def_nr(c);
            if c_def != u32::MAX {
                let content = self.data.def(c_def).known_type();
                // @P353: flush the content bare type if it is registered
                // after this wrapper struct (forward reference in id order).
                self.flush_bare_through(w, bare_io, bare_emitted, content)?;
                let content_ref = type_id_ref(content);
                emit_db_field(
                    w,
                    s_var,
                    field_name,
                    "vec",
                    &format!("db.vector({content_ref})"),
                )?;
            }
            return Ok(());
        }
        if let Type::Integer(int_spec) = typedef {
            // Post-2c: the field's size may come from the integer alias's
            // `size(N)` annotation (captured in `Attribute.alias_d_nr` →
            // `Data::forced_size`), from the width the TYPE itself carries, or
            // from the `Type::Integer` range.  Mirrors `fill_database`'s integer
            // arm in `src/typedef.rs` exactly, all three rungs, so the runtime
            // Parts matches the interpreter's (Byte/Short/Int/base).
            //
            // The middle rung is the one a SYNTHESIZED field needs.  `parse_field`
            // captures `alias_d_nr` for a field somebody declared, so `u8?` on a
            // struct resolves its width through the alias; a field built from a
            // Type alone — `__tuple<…>`'s elements, a mutated capture's `__cell_…`
            // value — has no alias, and without this rung the range heuristic
            // silently widens it (a nullable `u8` reserves its sentinel and becomes
            // a 2-byte `short`).  The compiler reads the width off the type and does
            // not widen, so the two disagreed about one field: `byte<0,true>` in the
            // compiler's table against `short<0,true>` in generated `init()`, which
            // renames every id past it (loft#739).  @PLN114 added the rung to the
            // interpreter for the tuple case; this is its native twin.
            let field_size = forced_size
                .or_else(|| int_spec.forced_size.map(std::num::NonZeroU8::get))
                .unwrap_or_else(|| typedef.size(nullable));
            // …including the OFFSET the Part carries: `part_min`, the same one the field's
            // ops encode against, which a nullable signed narrow field shifts by one.
            // Emitting the declared `min` here left the generated `init()` registering a
            // type the compiler no longer names, which loft#739's guard reports as a
            // schema-id divergence.
            let min = int_spec.part_min(field_size, nullable);
            debug_assert!(
                matches!(field_size, 1 | 2 | 4 | 8),
                "emit_field: unexpected integer field width \
                 field_size={field_size} for `{field_name}` — only 1/2/4/8 \
                 are supported by db.byte / db.short / db.int / db.field"
            );
            if field_size == 1 {
                emit_db_field(
                    w,
                    s_var,
                    field_name,
                    "byte",
                    &format!("db.byte({min}, {nullable})"),
                )?;
            } else if field_size == 2 {
                // Match the ONE width→op home (`NarrowIntKind::of(2, nullable, false)`): a
                // NULLABLE 2-byte field is `db.short` (the `+1` sentinel encoding), a NON-null
                // one is `db.short_raw` (direct — the `ShortFull` write is `OpSetShortRaw`).
                // Using `db.short` for a non-null field made the schema READ (`ShowDb`/to_json/
                // store round-trip) apply the `+1` shift the direct write never did → a non-null
                // `u16` field read back off-by-one / `i32::MIN` (interp fixed in typedef.rs; this
                // is the native db-setup twin).
                let (label, ctor) = if nullable {
                    ("short", format!("db.short({min}, {nullable})"))
                } else {
                    ("short_raw", format!("db.short_raw({min}, {nullable})"))
                };
                emit_db_field(w, s_var, field_name, label, &ctor)?;
            } else if field_size == 4 {
                emit_db_field(
                    w,
                    s_var,
                    field_name,
                    "int",
                    &format!("db.int({min}, {nullable})"),
                )?;
            } else {
                writeln!(w, "    db.field({s_var}, \"{field_name}\", 0);")?;
            }
            return Ok(());
        }
        if let Type::Sorted(c_nr, keys, _) = typedef {
            let c_tp = self.data.def(*c_nr).known_type();
            let c_ref = type_id_ref(c_tp);
            let keys_str = keys
                .iter()
                .map(|(k, asc)| format!("(\"{k}\".to_string(), {asc})"))
                .collect::<Vec<_>>()
                .join(", ");
            emit_db_field(
                w,
                s_var,
                field_name,
                "sorted",
                &format!("db.sorted({c_ref}, &[{keys_str}])"),
            )?;
            return Ok(());
        }
        if let Type::Hash(c_nr, keys, _) = typedef {
            let c_tp = self.data.def(*c_nr).known_type();
            let c_ref = type_id_ref(c_tp);
            let keys_str = keys
                .iter()
                .map(|k| format!("\"{k}\".to_string()"))
                .collect::<Vec<_>>()
                .join(", ");
            emit_db_field(
                w,
                s_var,
                field_name,
                "hash",
                &format!("db.hash({c_ref}, &[{keys_str}])"),
            )?;
            return Ok(());
        }
        // @PLN48 — a `spatial<T[x,y]>` field: keyed like Hash but the runtime
        // structure is a radix (Morton) tree.  Same key-name emission as Hash;
        // `db.spatial(content, keys)` builds the Radix Part.
        if let Type::Radix(c_nr, keys, _) = typedef {
            let c_tp = self.data.def(*c_nr).known_type();
            let c_ref = type_id_ref(c_tp);
            let keys_str = keys
                .iter()
                .map(|k| format!("\"{k}\".to_string()"))
                .collect::<Vec<_>>()
                .join(", ");
            emit_db_field(
                w,
                s_var,
                field_name,
                "spatial",
                &format!("db.spatial({c_ref}, &[{keys_str}])"),
            )?;
            return Ok(());
        }
        // A `trie<T[k]>` field: one text key, `db.trie(content, key)`.  Like the
        // keyed arms above it must be created INLINE here, or the field's type id
        // is never registered and its record layout reads as a struct with no
        // fields (`field_type` → index 0 of an empty list).
        if let Type::Trie(c_nr, key, _) = typedef {
            let c_tp = self.data.def(*c_nr).known_type();
            let c_ref = type_id_ref(c_tp);
            emit_db_field(
                w,
                s_var,
                field_name,
                "trie",
                &format!("db.trie({c_ref}, \"{key}\")"),
            )?;
            return Ok(());
        }
        if let Type::Index(c_nr, keys, _) = typedef {
            let c_tp = self.data.def(*c_nr).known_type();
            let c_ref = type_id_ref(c_tp);
            let keys_str = keys
                .iter()
                .map(|(k, asc)| format!("(\"{k}\".to_string(), {asc})"))
                .collect::<Vec<_>>()
                .join(", ");
            emit_db_field(
                w,
                s_var,
                field_name,
                "index",
                &format!("db.index({c_ref}, &[{keys_str}])"),
            )?;
            return Ok(());
        }
        if matches!(typedef, Type::Function(_, _, _)) {
            // Storage holds the 4-byte i32 d_nr.  When a capturing
            // closure was assigned to this attribute, the parser split
            // it into TWO database fields (`<attr>` +
            // `<attr>__closure_rec`, see `typedef.rs::fill_database`)
            // — mirror whichever shape the REGISTERED stores layout
            // carries so native and interpreter agree (#313).
            emit_db_field(w, s_var, field_name, "int", "db.int(0, false)")?;
            let crec_name = format!("{field_name}__closure_rec");
            if host_type_id != u16::MAX
                && let crate::database::Parts::Struct(fields)
                | crate::database::Parts::EnumValue(_, fields) =
                    &self.stores.types[host_type_id as usize].parts
                && let Some(f) = fields.iter().find(|f| f.name == crec_name)
                && let crate::database::Parts::ChildRec(c) =
                    self.stores.types[f.content as usize].parts
            {
                let c_ref = type_id_ref(c);
                emit_db_field(
                    w,
                    s_var,
                    &crec_name,
                    "crec",
                    &format!("db.child_rec({c_ref})"),
                )?;
            }
            return Ok(());
        }
        // Plan-22 phase 02c (P258 native fix, 2026-05-12): auto-Reference
        // attribute — when the dep list is non-empty, use the 12-byte
        // Parts::DbRef storage shape (`db.dbref()`) instead of the
        // inline-struct-bytes path below (which uses the inner struct's
        // known_type).  Mirrors `src/typedef.rs::fill_database`'s
        // `Type::Reference(_, ref deps) if !deps.is_empty()` branch so
        // native + interp agree on layout.  Without this, native
        // computes the closure record's auto-Reference field as
        // inline-bytes (size = inner struct's size) but interp writes
        // 12-byte DbRef bytes — the resulting size mismatch causes
        // `claim_child_rec`'s byte-copy to truncate at native's
        // smaller size and the lambda body reads garbage instead of
        // the captured DbRef.
        //
        // #682: mirror the ADOPTED / BORROWED split too.  Both shapes are 12
        // bytes, so getting it wrong costs no layout mismatch — it silently
        // gives native the interpreter's old over-free instead.
        if let Type::Reference(_, deps) = typedef
            && !deps.is_empty()
        {
            if deps.is_borrowed_share() {
                emit_db_field(w, s_var, field_name, "dbref_borrow", "db.dbref_borrow()")?;
            } else {
                emit_db_field(w, s_var, field_name, "dbref", "db.dbref()")?;
            }
            return Ok(());
        }
        if known_type != u16::MAX {
            let kt_ref = type_id_ref(known_type);
            writeln!(w, "    db.field({s_var}, \"{field_name}\", {kt_ref});")?;
        }
        Ok(())
    }

    /// Populate struct / enum-value fields, restricted to the given
    /// `phase`.  Runs once per struct per phase (Simple before any bare
    /// Sorted/Hash/Index types register, Collection after they do).
    #[allow(clippy::too_many_arguments)]
    fn output_struct_fields_filtered(
        &self,
        w: &mut dyn Write,
        def_nr: u32,
        enum_value: i32,
        type_id: u16,
        phase: FieldPhase,
        bare_io: &[(u16, BareIo)],
        bare_emitted: &mut [bool],
    ) -> std::io::Result<()> {
        let def = self.data.def(def_nr);
        let s_var = format!("t{type_id}");
        // Implicit enum-discriminator byte (inserted when the runtime
        // already had a plain `byte` type at the position where the
        // variant's content fields should begin).  Emitted only in
        // Phase 1 so field indices line up before any collection
        // fields are added.
        if phase == FieldPhase::Simple
            && enum_value > 0
            && def.known_type() != u16::MAX
            && self.stores.position(def.known_type(), "enum") == 0
        {
            writeln!(w, "    let byte_enum = db.byte(0, false);")?;
            writeln!(w, "    db.field({s_var}, \"enum\", byte_enum);")?;
        }
        for a in def.attributes() {
            let is_coll = is_collection_field(&a.typedef);
            let emit = match phase {
                FieldPhase::AllFields => true,
                FieldPhase::Simple => !is_coll,
                FieldPhase::Collection => is_coll,
                FieldPhase::EnumValues => false,
            };
            if !emit {
                continue;
            }
            let td_nr = self.data.type_def_nr(&a.typedef);
            let field_type_id = self.data.def(td_nr).known_type();
            assert_ne!(def_nr, u32::MAX, "Unknown def_nr for {:?}", a.typedef);
            let forced = self.data.forced_size(a.alias_d_nr);
            self.emit_field(
                w,
                &s_var,
                type_id,
                &a.name,
                &a.typedef,
                a.nullable,
                field_type_id,
                forced,
                bare_io,
                bare_emitted,
                &a.value,
            )?;
        }
        Ok(())
    }

    /// Use this to emit one loft function as a Rust function.
    /// Every loft function receives `stores: &mut Stores` as its first implicit argument.
    fn output_function(
        &mut self,
        w: &mut dyn Write,
        def_nr: u32,
        program_store: Option<&(Stores, crate::keys::DbRef)>,
    ) -> std::io::Result<()> {
        self.start_fn(def_nr);
        let def = self.data.def(def_nr);
        // Skip Op functions with no callable body.
        if def.name().starts_with("Op") && *def.code() == Value::Null {
            return Ok(());
        }
        // Skip functions implemented in codegen_runtime — emitting a stub
        // would shadow the real implementation.  Plan 09 phase 01
        // consolidated the hardcoded list into the registry in
        // `src/codegen_runtime.rs::CODEGEN_RUNTIME_FNS`.
        if *def.code() == Value::Null && crate::codegen_runtime::is_codegen_runtime_fn(def.name()) {
            return Ok(());
        }
        // N8b.1: generator functions (returning iterator<T>) are emitted as state machines.
        if matches!(def.returned(), Type::Iterator(_, _)) {
            return self.output_coroutine(w, def_nr);
        }
        // n_assert needs generic Display parameters to accept both Str and &str.
        if def.name() == "n_assert" && *def.code() == Value::Null {
            writeln!(
                w,
                "fn n_assert<M: std::fmt::Display, F: std::fmt::Display>(cell: &std::cell::UnsafeCell<Stores>, test: u8, msg: M, file: F, line: i64) {{"
            )?;
            // @PLN17: `test` is a boolean in storage form (u8); the assert fails
            // when it is not the true byte (1) — i.e. false (0) OR null (255).
            //
            // `report_and_exit`, the same path `n_panic` below takes, so the two explicit
            // halt statements read the same way on every backend.  It used to be a bare
            // `panic!`, which reached the user as `thread '<unnamed>' panicked at
            // /tmp/loft_native_2466316.rs:966` — a Rust location in a generated file the
            // author has never seen, where `--interpret` printed a loft diagnostic naming
            // their own source (loft#1056).  The first attempt at this swap was reverted
            // because the panic HOOK was what printed the loft call frames on the browser
            // target and `report_and_exit` printed none; the frames now come from the
            // shadow call stack inside `report_and_exit` itself, so nothing is traded away
            // — and `--interpret`, which had no frames here either, gains them too.
            // Production mode logs and continues (C66), through the same
            // `logged_in_production` the interpreter's `native.rs::n_assert` calls — the
            // two backends implement one documented behaviour and only the interpreter
            // used to honour it, so `loft app.loft` aborted where `loft --interpret`
            // carried on (loft#1263).
            writeln!(
                w,
                "  if test != 1 {{\n    let stores: &mut Stores = unsafe {{ &mut *cell.get() }};\n    let kind = loft::runtime_error::RuntimeErrorKind::AssertionFailed {{ message: msg.to_string() }};\n    if loft::runtime_error::logged_in_production(stores, &kind, &file.to_string(), line as u32) {{ return; }}\n    loft::runtime_error::RuntimeError::assertion_failed(msg.to_string(), file.to_string(), line as u32).report_and_exit();\n  }}"
            )?;
            writeln!(w, "}}\n")?;
            return Ok(());
        }
        // `n_panic` needs a real body for the same reason `n_assert` does: it has no
        // `#rust` template, so the generic path below would emit `fn n_panic(..) {}` — an
        // empty stub.  That is exactly what shipped, and it made `panic("msg")` a
        // COMPLETE no-op on the default backend: no message, no halt, exit 0, while
        // `--interpret` printed the error and exited 1.
        //
        // It reports and exits at the CALL SITE rather than setting `had_fatal` the way
        // the interpreter's `native.rs::n_panic` does, because there is no bytecode loop
        // here to notice the flag — the generated `main` checks it only after `n_main`
        // returns, by which point the whole program has run on past the panic.
        //
        // Generic `Display` params mirror `n_assert`: the message reaches this as a
        // `Str` or a `&str` depending on whether it is a literal or a formatted string.
        // The `log_*` family, for the same reason as `n_panic`: no `#rust` template, not
        // special-cased, so the generic path emitted `fn n_log_error(..) {}` and every
        // structured log call vanishes on `--native`.  With a logger configured that is
        // 2 records on `--interpret` and 0 on `--native`.  Silent loss of
        // diagnostics on the backend you actually deploy is where bugs hide, which is why
        // this is worth a body rather than an allow-list entry.
        //
        // `log_fatal` only logs — it carries no halt semantics (that is `panic`), so all
        // four map to the same shape at different severities.  Generic `Display` params
        // mirror `n_assert` / `n_panic`: the message arrives as `Str` or `&str`.
        if *def.code() == Value::Null
            && let Some(sev) = match def.name() {
                "n_log_info" => Some("Info"),
                "n_log_warn" => Some("Warn"),
                "n_log_error" => Some("Error"),
                "n_log_fatal" => Some("Fatal"),
                _ => None,
            }
        {
            writeln!(
                w,
                "fn {}<M: std::fmt::Display, F: std::fmt::Display>(cell: &std::cell::UnsafeCell<Stores>, msg: M, file: F, line: i64) {{",
                def.name()
            )?;
            writeln!(
                w,
                "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
            )?;
            writeln!(
                w,
                "  if let Some(ref logger) = stores.logger && let Ok(mut lg) = logger.lock() {{ lg.log(loft::logger::Severity::{sev}, &file.to_string(), line as u32, &msg.to_string()); }}"
            )?;
            writeln!(w, "}}\n")?;
            return Ok(());
        }
        if def.name() == "n_panic" && *def.code() == Value::Null {
            writeln!(
                w,
                "fn n_panic<M: std::fmt::Display, F: std::fmt::Display>(cell: &std::cell::UnsafeCell<Stores>, msg: M, file: F, line: i64) {{"
            )?;
            // Same shared decision as `n_assert` above (loft#1263).
            writeln!(
                w,
                "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};\n  let kind = loft::runtime_error::RuntimeErrorKind::UserPanic {{ message: msg.to_string() }};\n  if loft::runtime_error::logged_in_production(stores, &kind, &file.to_string(), line as u32) {{ return; }}\n  loft::runtime_error::RuntimeError::user_panic(msg.to_string(), file.to_string(), line as u32).report_and_exit();"
            )?;
            writeln!(w, "}}\n")?;
            return Ok(());
        }
        // DX-source-map: emit a `// loft:<file>:<line>` comment
        // above each function so rustc errors at the function header
        // (e.g. wrong arg type, missing trait impl) map directly to
        // the .loft definition site.
        if !def.position().file.is_empty() {
            writeln!(w, "// loft:{}:{}", def.position().file, def.position().line)?;
        }
        write!(
            w,
            "{}fn {}(cell: &std::cell::UnsafeCell<Stores>",
            self.fn_inline_attr(),
            self.fn_ident(def)
        )?;
        for a in def.attributes() {
            let tp = rust_type(&a.typedef, &Context::Argument);
            write!(w, ", mut var_{}: {tp}", sanitize(&a.name))?;
        }
        write!(w, ") ")?;
        if *def.returned() != Type::Void {
            // @PLN10 — owned-`String` vs buffer-backed `Str` wrapper: the single
            // decision lives in `returns_owned_string` (shared with the
            // shared-store bridge), so the signature and the body never disagree.
            if returns_owned_string(def) {
                write!(w, "-> String ")?;
            } else {
                write!(w, "-> {} ", rust_type(def.returned(), &Context::Result))?;
            }
        }
        // Mark argument variables as already declared so Set won't re-declare them.
        for arg_nr in def.variables().arguments() {
            self.declared.insert(arg_nr);
        }
        // #260 Fix B: declare the owning `__vdb` store locals UP FRONT, from
        // the variable table, decoupling the `let` position from the IR
        // null-init position — `lastuse_reclaim` may then relocate a
        // null-init below an early-return scope-exit free without stranding
        // the free's `var_…` reference out of scope (rustc E0425, 92× on the
        // pre-Fix-A brick-buster `--html` build).  The prologue binds the
        // NULL SENTINEL only (no allocation; `OpFreeRef` no-ops on it); the
        // first body `Set(v, Null)` still emits `null_named` + `OpDatabase`
        // at its IR position via `predeclared`.
        let mut vdb_prologue = String::new();
        {
            let vars = def.variables();
            // loft#1178 — and every VIEW a `return` NAMES, for the reason loft#731 gives one
            // paragraph down about the iteration scratch: a Rust `let` lives where the
            // emission first reaches it, and a `return` at function level naming a local whose
            // `let` landed inside a nested block is `E0425: cannot find value in this scope`.
            // The interpreter cannot have this — every local is a frame slot — so it is a
            // property of the EMISSION, not of the program, and the cure is the same one the
            // two names below already get: bind it up front so the `let` position stops
            // deciding the scope.  The map desugar's `_map_result_1` is the shape that made
            // this general: it is built inside the comprehension block and handed back from
            // outside it.
            //
            // A VIEW only — a local whose type NAMES what it borrows.  #354 measured the
            // other half and its sentence is the boundary: hoisting a heap local that OWNS
            // its store re-inits a fresh one per call that the matched free no longer covers,
            // and the crawler's libs exhausted the store table on it.  A view owns nothing,
            // so a `DbRef::NULL` pre-binding can orphan nothing; the store it looks into is
            // hoisted by the `__vdb` rule above.
            let mut returned_vars: Vec<u16> = Vec::new();
            Self::collect_returned_vars(def.code(), &mut returned_vars);
            for v in 0..vars.count() {
                // loft#731 — the iteration scratch belongs here for exactly the
                // reason above, and was missed because it arrives by a different
                // route. `scopes.rs` gives every `hash_scratch` var an
                // `OpFreeScratch` at the FUNCTION's scope exit (a `return` out
                // of an exposed loop bypasses the loop epilogue), while its
                // null-init is placed wherever the variable is first used — an
                // `else if` branch puts that inside a nested block, and the
                // function-level free then names a binding that is out of scope.
                // Same E0425, same cure: bind the sentinel up front so the `let`
                // position stops depending on where the null-init landed.
                let is_iter_scratch = vars.name(v).contains("hash_scratch");
                if !vars.is_argument(v)
                    && (vars.name(v).starts_with("__vdb")
                        || is_iter_scratch
                        || (returned_vars.contains(&v) && !vars.tp(v).depend().is_empty()))
                    && rust_type(vars.tp(v), &Context::Variable) == "DbRef"
                {
                    use std::fmt::Write as _;
                    let _ = write!(
                        vdb_prologue,
                        "\n  let mut var_{}: DbRef = DbRef::NULL;",
                        sanitize(vars.name(v))
                    );
                    self.declared.insert(v);
                    self.predeclared.insert(v);
                }
            }
            // Entry-buffer witness for each hidden return buffer (retbuf): stash
            // the caller's buffer at function entry as `_rb_w_<name>`.  A
            // CONDITIONAL reassignment of the return-local (`chosen = m_none();
            // if c { chosen = … }`) then frees the orphaned fn-owned intermediate
            // guarded by `_old != _rb_w_<name>`, so it never frees the caller's
            // buffer — closing the cluster-462 native record leak (the interp
            // already frees the orphan; native's reassign-free excluded the
            // retbuf-attr entirely).  Leading `_` suppresses the unused warning
            // for retbuf locals that are never reassigned.
            for a in def.attributes() {
                if a.hidden && matches!(&a.typedef, Type::Reference(_, _) | Type::Enum(_, true, _))
                {
                    let av = vars.var(&a.name);
                    if av != u16::MAX && (av as usize) < vars.count() as usize {
                        use std::fmt::Write as _;
                        let nm = sanitize(vars.name(av));
                        let _ = write!(vdb_prologue, "\n  let _rb_w_{nm}: DbRef = var_{nm};");
                        self.retbuf_witness.insert(av);
                    }
                }
            }
            // @PLN90 #495 — the runtime-Join owned-store tracker.  For each
            // "runtime-Join" local (owned init + ≥1 ncc-borrow reassign) declare
            // `_own_store_<name>: DbRef` (NULL sentinel).  `output_set` keeps it
            // pointed at the store r actually OWNS (var_r after an owned assign,
            // NULL once r holds a borrow); the in-loop displaced-free and the
            // scope-exit `OpFreeRef` (ops/ref_ops.rs) free THIS, never r's view.
            self.witness_vars = collect_witness_vars(self.data, def_nr);
            // Emit in variable order, not hash order.  `witness_vars` is a `HashSet`,
            // whose iteration order Rust randomises PER PROCESS, so emitting straight
            // from it made the generated Rust differ between two runs of the same binary
            // on the same input — the declarations are independent, so the program was
            // always correct, but a `--native` build was not reproducible and any
            // before/after diff of generated source was unusable as an instrument.
            let mut witnesses: Vec<u16> = self.witness_vars.iter().copied().collect();
            witnesses.sort_unstable();
            for wv in witnesses {
                use std::fmt::Write as _;
                let nm = sanitize(vars.name(wv));
                let _ = write!(
                    vdb_prologue,
                    "\n  let mut _own_store_{nm}: DbRef = DbRef::NULL;"
                );
            }
            // #354: hoist block-crossing locals into the prologue — loft
            // locals are function-scoped frame slots, so a `let` at the
            // first write inside a nested block loses the variable for
            // sibling-block uses (E0425).  Same declared/predeclared
            // mechanics as the `__vdb` prologue above.
            // #354: hoist ONLY plain SCALAR locals (integer / float / single
            // / boolean / character / non-struct enum).  Those are the
            // block-split E0425 cases (`intown`, `nhouse`, `tsize`, `ax`,
            // `cwx` …) and a scalar prologue default is a pure value — no
            // store, no ownership.  Heap locals (DbRef / Text / Vector …)
            // are deliberately EXCLUDED: their `let` carries store/free
            // ownership the `__vdb` prologue + scope analysis already place,
            // and hoisting them re-init'd a fresh store per call that the
            // matched free no longer covered (a store leak — crawler's
            // hex/sim libs exhausted the 65535-store table).
            // Sorted for the same reason as the witness prologue above: the collector
            // answers a `HashSet`, and emitting in its iteration order made two runs of
            // one binary produce different Rust.
            let mut hoists: Vec<u16> = collect_scope_hoists(def.code()).into_iter().collect();
            hoists.sort_unstable();
            for v in hoists {
                if self.declared.contains(&v) || vars.is_argument(v) || v >= vars.count() {
                    continue;
                }
                let tp = vars.tp(v);
                // One home: `data::is_scalar` (formal/IMPLEMENTATIONS.md #1).
                if !crate::data::is_scalar(tp) {
                    continue;
                }
                let tp_str = rust_type(tp, &Context::Variable);
                use std::fmt::Write as _;
                let _ = write!(
                    vdb_prologue,
                    "\n  let mut var_{}: {tp_str} = {};",
                    sanitize(vars.name(v)),
                    default_native_value(tp),
                );
                self.declared.insert(v);
            }
        }
        // Determine the user-visible loft name for the shadow call stack.
        let loft_name = def.name().strip_prefix("n_").unwrap_or(def.name());
        let loft_file = &def.position().file;
        let loft_line = def.position().line;
        // Only instrument user-defined functions (Block body, n_ prefix).
        let instrument = matches!(def.code(), Value::Block(_)) && def.name().starts_with("n_");
        let returns_text = matches!(def.returned(), Type::Text(_));
        if let Value::Block(bl) = def.code() {
            // An empty-body loft function (explicit stub) has no operators and result Void,
            // but the function signature may still declare a non-void return type.
            // Rust requires an explicit return value in that case, so emit a null default.
            let block_empty = bl.operators.iter().all(|v| matches!(v, Value::Line(_)));
            // @PLN11 G2/M2/M5 — store-backed body emission.  When
            // output_functions supplied a persistent program store, read this
            // body's node from it (def_body_node) and emit through IrBlock::Store
            // (output_block materialises at its boundary); the generated Rust is
            // identical to native.  No store → IrBlock::Native.
            let body: IrBlock = if let Some((stores, root)) = program_store {
                IrBlock::Store(stores, crate::ir_read::def_body_node(stores, *root, def_nr))
            } else {
                IrBlock::Native(bl)
            };
            if block_empty && *def.returned() != Type::Void {
                writeln!(w, "{{")?;
                writeln!(
                    w,
                    "  let _stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
                )?;
                // An empty body assigns nothing, so the return takes the type's DEFAULT —
                // not its null (@FR-E-Uncomp-NN, loft#1254).
                //
                // Which SPELLING that default takes is `returns_owned_string`'s decision,
                // and the signature above already asked it; its own doc is the contract —
                // "the single decision lives in `returns_owned_string` … so the signature
                // and the body never disagree".  Asking it here too is what stops a `-> text`
                // stub emitting a `Str` under a `-> String` signature, which rustc rejects
                // (E0308): a raw compiler error reaching the user for a program loft
                // accepted (loft#1254).
                let ctx = if returns_owned_string(def) {
                    Context::Variable
                } else {
                    Context::Result
                };
                writeln!(w, "  {}", uninitialised_native_value(def.returned(), &ctx))?;
                writeln!(w, "}}")?;
            } else if instrument {
                // Emit shadow call stack instrumentation before the block body.
                // The CallGuard drop ensures cr_call_pop on all exit paths (including early return).
                // We emit the push/guard as a prefix inside the block's opening `{`.
                // P199 — prepend the `&mut Stores` derivation from the
                // `&UnsafeCell<Stores>` parameter so templates and inner
                // emissions see `stores` as a regular `&mut Stores` binding.
                let escaped_file = loft_file.replace('\\', "\\\\");
                // @PLN18 08-S2 — the live-dispatch entry check precedes even
                // the `stores` derivation: a flipped fn re-enters the parked
                // interpreter (which swaps the world out of the cell), so no
                // native `&mut` may be live in THIS frame when it runs.
                // @PLN98 P2 — `--lean` strips the tier: skip the entry check
                // entirely (and, as a side effect, `self.live_fns` stays empty
                // because `live_entry_check` — its sole producer — never runs).
                let live_check = if self.emit_live {
                    self.live_entry_check(def).unwrap_or_default()
                } else {
                    String::new()
                };
                // loft#1183 — a fn-ref dispatch arm that pre-allocates a vector return
                // buffer hands the DELIVERED one to a destination whose static type may
                // say it owns nothing, so the buffer needs the frame as its owner.  Every
                // instrumented frame carries the guard, not only the ones that ALLOCATE a
                // buffer: a heap-returning frame passes its buffers to its caller, so the
                // frame that ends up releasing one is usually not the frame that made it,
                // and a gap in the chain is a store nothing frees.  A frame with none pays
                // two `Cell` reads (`codegen_runtime::FnRefBufGuard`).
                let hands_up = matches!(
                    def.returned().base(),
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                );
                let fnref_guard = format!(
                    "\n  let _fnref_guard = codegen_runtime::FnRefBufGuard::new(cell, {hands_up});"
                );
                self.call_stack_prefix = Some(format!(
                    "{live_check}  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};\n  \
                     cr_call_push(\"{loft_name}\", \"{escaped_file}\", {loft_line});\n  \
                     let _call_guard = codegen_runtime::CallGuard;{fnref_guard}{vdb_prologue}"
                ));
                self.output_block(w, body, returns_text, true)?;
                self.call_stack_prefix = None;
            } else {
                // Non-instrumented user-fn (e.g. `t_…` methods) — still
                // needs the `&mut Stores` derivation from the UnsafeCell
                // parameter for templates / inner calls.
                self.call_stack_prefix = Some(format!(
                    "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};{vdb_prologue}"
                ));
                self.output_block(w, body, returns_text, true)?;
                self.call_stack_prefix = None;
            }
        } else if *def.code() == Value::Null {
            // Native-only function with no loft body.
            // @PLAN12 phase 2 step 2 (2026-05-24): check `def.native` FIRST.
            // The manifest's `[native.functions]` table now populates
            // `def.native` for matching defs (via parser/mod.rs's
            // `register_native_manifest` and `apply_manifest_side_effects`),
            // so the `def.native` path covers everything the `native_symbols`
            // path used to.  The `native_symbols` branch is kept as a fallback
            // for the legacy case where someone declares `[native.functions]`
            // without an `[library] native = "..."` stem — that yields a
            // populated `native_symbols` map but no crate name, so the
            // `def.native`-driven call (which qualifies via
            // `native_symbol_crates`) would emit an unqualified symbol.  The
            // legacy path's `output_native_api_call` emits `{sym}(stores, …)`
            // with no crate qualifier, so it only works when the symbol is
            // either in the current crate or already imported.  Today the
            // primary callers always have a crate, so the def.native path
            // wins.
            let user_name = def.name().strip_prefix("n_").unwrap_or(def.name());
            if !def.c_sig.is_empty() {
                // @PLN24 arc C — a `#c` binding: call the C symbol directly,
                // through the typed extern the header declared.  Checked FIRST
                // because a `#c` def never carries `native` (arc A leaves it
                // empty on purpose, so the Rust dispatch path cannot claim it).
                self.output_c_direct_call(w, def_nr)?;
            } else if !def.native().is_empty() {
                // #native "symbol" — emit direct call with type marshalling.
                if self.wasm_browser {
                    // #623 — on the browser target a `#native` symbol reaches its
                    // implementation through `[wasm.bridge].routes`.  With no route
                    // this call has nothing to bind to, and the emitted crate used
                    // to carry both a host-import declaration and this wrapper under
                    // one name — a raw rustc E0428/E0610/E0061 cascade that named
                    // neither the symbol's library nor `[wasm.bridge.routes]`.  Say
                    // so directly instead; `emit_file_header` drops the colliding
                    // extern so this is the only error rustc reports.  Reachability-
                    // scoped, matching P269 / @PLN26: an unused routeless declaration
                    // must not reject an otherwise-valid program.
                    let reachable = self.reachable.is_empty() || self.reachable.contains(&def_nr);
                    let stores_loft_ref =
                        matches!(def.returned(), Type::Vector(_, _) | Type::Reference(_, _));
                    let has_ref_arg = def.attributes().iter().any(|a| {
                        !a.name.starts_with("__")
                            && a.typedef.heap_dep().is_some()
                            && !matches!(a.typedef, Type::Vector(_, _))
                    });
                    // The store-mutating shapes keep their graceful @P321c stub —
                    // they never had a host import to collide with.
                    let host_import_shape = !stores_loft_ref && !has_ref_arg;
                    // Only a BARE `#native` collides: its symbol is the loft fn's
                    // own emitted name, so the host-import declaration and this
                    // wrapper claim one name.  An explicit differing symbol
                    // (`#native "loft_gl_swap_buffers"`) is the working raw
                    // host-import path — leave it alone.
                    let symbol_is_own_name = def.native() == def.name();
                    // A `text` RETURN cannot cross a raw host import either: the
                    // extern is declared `-> i32` while the wrapper reads a `Str`
                    // off it (`_ret_str.ptr` → E0610).  Same missing-bridge cause,
                    // so it earns the same message rather than that cascade.
                    let returns_text = matches!(def.returned(), Type::Text(_));
                    if reachable
                        && host_import_shape
                        && (symbol_is_own_name || returns_text)
                        && !self.data.wasm_bridge_routes.contains_key(def.native())
                    {
                        let from = self
                            .data
                            .native_symbol_crates
                            .get(def.native())
                            .map_or_else(String::new, |k| format!(" (from library `{k}`)"));
                        // User fns are stored as `n_<name>`; report the name as written.
                        let loft_name = def.name().strip_prefix("n_").unwrap_or(def.name());
                        writeln!(
                            w,
                            "{{ compile_error!(\"loft --html: `#native \\\"{}\\\"`{} has no [wasm.bridge].routes entry, so `{}` cannot be called from the browser (--html / --native-wasm) target. Add `{} = \\\"<bridge_fn>\\\"` under [wasm.bridge.routes] in that library's loft.toml, or avoid this function on the browser target.\") }}",
                            def.native(),
                            from,
                            loft_name,
                            def.native()
                        )?;
                    } else {
                        // wasm host import — unqualified; declared in the preamble via
                        // `#[link(wasm_import_module = "loft_gl")]`.
                        self.output_native_direct_call(w, def_nr, def.native())?;
                    }
                } else if let Some(krate) = self.data.native_symbol_crates.get(def.native()) {
                    if self.native_cabi {
                        // @PLN26 phase 1 — a `#native` symbol exported by 2+ packages
                        // can't be disambiguated across the flat C-ABI namespace
                        // (the link resolves first-`.so`-wins).  Reject it ONLY here,
                        // at a REACHABLE call site — so two packages sharing an
                        // unused symbol still build, and the error names a real call.
                        let reachable =
                            self.reachable.is_empty() || self.reachable.contains(&def_nr);
                        if reachable && self.native_collisions.contains(def.native()) {
                            writeln!(
                                w,
                                "{{ compile_error!(\"loft --native: native packages export the same #native symbol '{}'; the C-ABI link cannot disambiguate them — rename one with #native \\\"<unique-symbol>\\\" (@PLN26 phase 1)\") }}",
                                def.native()
                            )?;
                        } else {
                            // C-ABI: call the symbol via its `__cabi_`-prefixed alias
                            // (declared with `#[link_name]` in the `extern "C"` block),
                            // resolved by linking the package's cdylib `.so` — no
                            // `extern crate` (NATIVE.md § Resolution).  The alias avoids
                            // shadowing the same-named wrapper fn (E0428).
                            let aliased = format!("__cabi_{}", def.native());
                            self.output_native_direct_call(w, def_nr, &aliased)?;
                        }
                    } else {
                        // rlib path (Windows / cross-compiled targets): name the
                        // implementing fn in the package crate, which for a
                        // remapped binding differs from the `#native` string
                        // (loft#907).
                        let qualified =
                            format!("{}::{}", krate, self.data.link_symbol(def.native()));
                        self.output_native_direct_call(w, def_nr, &qualified)?;
                    }
                } else {
                    // P269: refuse to emit a runtime panic for a reachable
                    // unimplemented native — convert to a compile-time error
                    // per the "fail at startup, not runtime" principle.
                    // Unreachable defs keep the `todo!()` shim so unused
                    // declarations don't reject otherwise-valid programs.
                    //
                    // The RETURN TYPE is not part of that question (loft#993). It used to
                    // gate the whole escalation, so a VOID unimplemented native was
                    // emitted as `{}` — a function that compiles, is callable, and does
                    // nothing. "Fail at startup, not runtime" has no effect on the half
                    // of the surface where the failure is silent instead of a panic.
                    let reachable = self.reachable.is_empty() || self.reachable.contains(&def_nr);
                    writeln!(w, "{{")?;
                    if reachable {
                        writeln!(
                            w,
                            "  compile_error!(\"loft --native: native fn `{}` (#native \\\"{}\\\") has no implementation in any registered native crate; either run via --interpret or wire the symbol in a #native package or src/codegen_runtime.rs (P269)\")",
                            def.name(),
                            def.native()
                        )?;
                    } else {
                        writeln!(w, "  todo!(\"native function {}\")", def.name())?;
                    }
                    writeln!(w, "}}")?;
                }
            } else if let Some(rust_symbol) = self.data.native_symbols.get(user_name) {
                // Fallback: `[native.functions]` declared the mapping but no
                // `[library] native = "..."` stem populated `def.native`.
                // Emits unqualified `{rust_symbol}(stores, …)` — caller must
                // ensure the symbol is in scope (either same crate or
                // explicitly imported).  Primary callers always have a crate
                // (handled by the def.native path above); this is for
                // future, less-conventional configurations.
                self.output_native_api_call(w, def_nr, rust_symbol)?;
            } else {
                // Internal i_ functions have implementations in codegen_runtime.rs;
                // all others get a todo!() stub.
                writeln!(w, "{{")?;
                if def.name() == "i_parse_errors" {
                    writeln!(
                        w,
                        "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
                    )?;
                    writeln!(w, "  loft::codegen_runtime::i_parse_errors(stores)")?;
                } else if def.name() == "i_parse_error_push" {
                    writeln!(
                        w,
                        "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
                    )?;
                    writeln!(
                        w,
                        "  loft::codegen_runtime::i_parse_error_push(stores, var_msg)"
                    )?;
                } else if def.name() == "n_json_errors" {
                    writeln!(
                        w,
                        "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
                    )?;
                    writeln!(w, "  loft::codegen_runtime::i_json_errors(stores)")?;
                } else {
                    // P269: same compile-time-error escalation as above for
                    // reachable unimplemented natives without a `#native`
                    // annotation.  Unreachable internal stubs (e.g. unused
                    // `i_*` helpers) keep the `todo!()` shim.
                    //
                    // The RETURN TYPE used to gate this whole branch, so a VOID
                    // unimplemented built-in was emitted as `{}` — callable, silent, and
                    // doing nothing, where a value-returning one is loud.  That
                    // asymmetry is what hid the par discard route's missing lowering for
                    // its whole life: `--native` emitted `n_parallel_discard`'s
                    // declaration with an empty body, and only an unrelated arity
                    // mismatch made it visible (loft#993, loft#987).
                    //
                    // Nothing else here had to change, because `reachable` below ALREADY
                    // answers "is this def actually called through its declaration" — it
                    // excludes a `#rust` body (inlined at the call site), an interface or
                    // T-param stub, and a def with a registered `OpEmitter` (whose call
                    // sites are rewritten).  A deliberate no-op therefore has to SAY so,
                    // and `yield_frame` — the one built-in that was relying on the
                    // silence — now carries `#rust "()"`.
                    //
                    // Three categories of "abstract declarations never
                    // called at runtime" must NOT trigger compile_error
                    // even when the reachability walker counts them:
                    //
                    // 1. Functions with a `#rust"…"` annotation (e.g.
                    //    text methods like `starts_with` / `ends_with`
                    //    / `trim` in `default/03_text.loft`) inline the
                    //    Rust expression at every call site via the
                    //    dispatch in `src/generation/calls.rs`.
                    // 2. Interface method stubs (`__iface_<N>_<method>`)
                    //    are abstract declarations created by
                    //    `parse_interface`; bound-generic dispatch
                    //    substitutes the call with the concrete impl
                    //    via `re_resolve_call`.
                    // 3. T-parameterized stubs (`t_<LEN><Tname>_<method>`
                    //    where `<Tname>` is a generic type variable)
                    //    are synthesized at function-parse time
                    //    (parse_function I7/I8.1); same re_resolve_call
                    //    substitution applies.
                    //
                    // For all three, the function body is emitted but
                    // never entered at runtime.  Use `todo!()` (the
                    // safe placeholder that costs nothing unless
                    // someone takes the address of the fn) instead
                    // of `compile_error!()`.
                    let is_iface_stub = def.name().starts_with("__iface_");
                    let is_t_stub = is_t_param_stub(def.name());
                    let has_custom_op_emitter = ops::has_custom_emitter(def.name());
                    let reachable = (self.reachable.is_empty() || self.reachable.contains(&def_nr))
                        && def.rust().is_empty()
                        && !is_iface_stub
                        && !is_t_stub
                        && !has_custom_op_emitter;
                    if reachable {
                        writeln!(
                            w,
                            "  compile_error!(\"loft --native: built-in fn `{}` has no native implementation; wire it in src/codegen_runtime.rs or run via --interpret (P269)\")",
                            def.name()
                        )?;
                    } else {
                        writeln!(w, "  todo!(\"native function {}\")", def.name())?;
                    }
                }
                writeln!(w, "}}")?;
            }
        } else {
            writeln!(w, "{{")?;
            writeln!(
                w,
                "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
            )?;
            // #260 Fix B: this branch (non-`Block` body shapes) bypasses
            // `output_block`, so the prologue cannot ride
            // `call_stack_prefix` — emit it directly or the `__vdb`
            // declarations marked in `declared` above never materialise
            // (E0425 on their frees).
            if !vdb_prologue.is_empty() {
                writeln!(w, "{}", vdb_prologue.trim_start_matches('\n'))?;
            }
            self.output_code_inner(w, def.code())?;
            writeln!(w, "\n}}")?;
        }
        writeln!(w, "\n")
    }

    /// PKG.4: emit a call to an external native Rust function from a package.
    /// The generated code calls `rust_symbol(stores, arg1, arg2, ...)` and
    /// returns the result.
    fn output_native_api_call(
        &self,
        w: &mut dyn Write,
        d_nr: u32,
        rust_symbol: &str,
    ) -> std::io::Result<()> {
        let def = self.data.def(d_nr);
        writeln!(w, "{{")?;
        writeln!(
            w,
            "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
        write!(w, "  {rust_symbol}(stores")?;
        for attr in def.attributes() {
            if attr.name.starts_with("__") {
                continue;
            }
            write!(w, ", var_{}", sanitize(&attr.name))?;
        }
        write!(w, ")")?;
        writeln!(w, "\n}}")
    }

    /// Emit a direct call to a native `extern "C"` function with automatic
    /// type marshalling derived from the loft function signature.
    ///
    /// Conversions:
    /// - `text` (`&str`) → `ptr, len` (two C args)
    /// - `vector<T>` → `(*const ELEM_TYPE, count: u32)` pair via direct store access
    /// - `text` → `(ptr, len)` pointer pair
    /// - scalars pass through with casts where needed
    ///
    /// `vector<T>` args never use `LoftStore`/`LoftRef`.  Instead the codegen
    /// extracts the raw element pointer and count from the store's memory buffer
    /// directly.  This avoids the E0308 "two different loft_ffi" error that arises
    /// when loft and the native package are compiled as separate Cargo projects.
    ///
    /// Native functions that take `vector<T>` args must declare their C signature
    /// with `(*const ELEM_TYPE, count: u32)` pairs in place of each vector argument
    /// (no `LoftStore` or `LoftRef` involved).
    ///
    /// The return value is converted back to the loft type.
    /// @PLN24 arc C — emit the body of a `#c` binding: marshal the loft
    /// arguments into the declared C types, call the symbol through the typed
    /// extern, and bring the result back.
    ///
    /// Separate from [`Self::output_native_direct_call`] rather than folded
    /// into it, because the two marshal genuinely different things and folding
    /// them would be a false unification. A `#native` text argument crosses as
    /// `ptr, len`; a C one has to be NUL-terminated. A `#native` symbol takes a
    /// `LoftStore` handle and reads a struct through it; a C symbol has no such
    /// API, which is exactly why a loft record is refused at the declaration.
    ///
    /// The RETURN is where the declared width earns its keep: the extern is
    /// declared at the C width, so rustc truncates at the ABI boundary and the
    /// `as i64` below extends correctly. -1 from an `int` stays -1 rather than
    /// becoming 4294967295.
    fn output_c_direct_call(&self, w: &mut dyn Write, d_nr: u32) -> std::io::Result<()> {
        let def = self.data.def(d_nr);
        let target = crate::c_signature::CTarget::host();
        // The declaration already reported a parse failure; emit a body anyway so
        // the wrapper compiles into a coherent error rather than an E0308 cascade.
        let Some(Ok(sig)) = crate::c_signature::of(self.data, d_nr, target) else {
            writeln!(
                w,
                "{{ compile_error!(\"loft --native: this `#c` binding's signature did not parse\") }}"
            )?;
            return Ok(());
        };
        if self.no_c_abi() {
            // @PLN24 arc E — the refusal, at the only site that knows a `#c`
            // binding is actually CALLED.  Reachability-scoped for free, which
            // is the property that matters: a library declaring bindings a wasm
            // program never uses still builds for wasm.
            //
            // The three ways this used to end, all measured on one tree: a
            // symbol the wasm sysroot happens to export LINKED and then trapped
            // (`signature_mismatch: strlen`); one it does not export produced a
            // raw `rust-lld: undefined symbol` naming neither package nor
            // library; and an optional-library binding produced `E0433: cannot
            // find c_call in loft`, once per symbol, pointing at loft's own
            // internals.  A reader could not tell any of the three from a bug in
            // their own program.
            let name = def.name().strip_prefix("n_").unwrap_or(def.name());
            let (which, flag) = if self.wasm_browser {
                ("browser", "--html")
            } else {
                ("wasm (wasip2)", "--native-wasm")
            };
            // The PACKAGE, not the library.  A `#c` annotation never names the
            // library it came from (@PLN24 arc G), so listing the package's
            // `[c]` entries would name several libraries of which at most one is
            // this symbol's — and one of them is the shim loft built itself,
            // which arc D deliberately made indistinguishable from a declared
            // one.  The package is the thing the author can act on.
            let from =
                self.data
                    .c_owner_pkg(&def.position().file)
                    .map_or_else(String::new, |pkg| {
                        let stem = std::path::Path::new(pkg)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(pkg);
                        format!(" (package `{stem}`)")
                    });
            writeln!(
                w,
                "{{ compile_error!(\"loft: `{name}` is bound to the C symbol '{}' with #c{from}, and the {which} target has no C ABI to reach it — a wasm module cannot open a shared library. Give the library a wasm implementation, host it out of process (@PLN119), or drop the {flag} claim (@PLN24 arc E)\") }}",
                sig.symbol
            )?;
            return Ok(());
        }

        let params: Vec<&crate::data::Attribute> = def
            .attributes()
            .iter()
            .filter(|a| !a.name.starts_with("__") && !a.name.starts_with('#'))
            .collect();
        let needs_stores = params
            .iter()
            .any(|a| matches!(a.typedef.base(), Type::Vector(_, _)));
        // Which C slots each loft parameter occupies — the same answer the
        // interpreter's `dispatch` reads, so a vector carries a count on both
        // backends or on neither (@PLN128 arc D).
        let loft_params: Vec<crate::data::Type> =
            params.iter().map(|a| a.typedef.clone()).collect();
        let Ok(arg_plan) = crate::c_signature::plan(&loft_params, &sig) else {
            // Unreachable in practice: the declaration check runs the same
            // `plan` and its error aborts before any native source is written.
            // Emitted as `compile_error!` rather than defaulted to an empty
            // plan, because an empty plan would emit a call with NO arguments
            // against a typed `extern "C"` — a rustc cascade pointing at loft's
            // generated source instead of at the declaration. Same shape as the
            // signature-did-not-parse arm above.
            writeln!(
                w,
                "{{ compile_error!(\"loft --native: this `#c` binding's parameters do not fit its C signature\") }}"
            )?;
            return Ok(());
        };

        writeln!(w, "{{")?;
        if needs_stores {
            writeln!(
                w,
                "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
            )?;
        }
        // Prologue: every argument that needs a temporary to exist for the
        // duration of the call. Bound here, in the wrapper's own block, so the
        // buffers outlive the call rather than dropping at the end of the call
        // expression.
        for (attr, arg) in params.iter().zip(&arg_plan) {
            let var = sanitize(&attr.name);
            match attr.typedef.base() {
                Type::Text(_) => {
                    // loft text is UTF-8 PLUS A LENGTH; C reads to the first
                    // NUL. The copy is what makes them the same thing, and it
                    // is per-call rather than cached because the callee may
                    // hold the pointer for its duration.
                    writeln!(
                        w,
                        "  let _cs_{var}: Vec<u8> = {{ let p = var_{var}.as_ptr(); let n = var_{var}.len(); let mut b = Vec::with_capacity(n + 1); if !p.is_null() && n > 0 {{ b.extend_from_slice(unsafe {{ std::slice::from_raw_parts(p, n) }}); }} b.push(0u8); b }};"
                    )?;
                }
                Type::Vector(elem_tp, _) => {
                    let elem = Self::vector_elem_rust_type(elem_tp);
                    writeln!(
                        w,
                        "  let _vr_{var} = loft::keys::store(&var_{var}, &stores.allocations).get_u32_raw(var_{var}.rec, var_{var}.pos);"
                    )?;
                    if *arg == crate::c_signature::CArg::VectorPtrCount {
                        writeln!(
                            w,
                            "  let _vc_{var} = if _vr_{var} == 0 {{ 0u32 }} else {{ loft::keys::store(&var_{var}, &stores.allocations).get_u32_raw(_vr_{var}, 4) }};"
                        )?;
                    }
                    // The elements live in a loft store the allocator may grow,
                    // so this pointer is valid FOR THE CALL and no longer — the
                    // contract the declaration's mapping table states.
                    writeln!(
                        w,
                        "  let _vp_{var}: *const {elem} = if _vr_{var} == 0 {{ std::ptr::null() }} else {{ loft::keys::store(&var_{var}, &stores.allocations).addr::<{elem}>(_vr_{var}, 8) as *const {elem} }};"
                    )?;
                }
                _ => {}
            }
        }

        // The call itself, argument by argument, against the DECLARED C types.
        // Which slots each loft parameter occupies comes from
        // `c_signature::plan`, the same answer the interpreter's `dispatch`
        // reads — a vector carries a count only where the signature has a
        // parameter for it (@PLN128 arc D).
        let mut args: Vec<String> = Vec::new();
        let mut c_i = 0usize;
        for (attr, arg) in params.iter().zip(&arg_plan) {
            let var = sanitize(&attr.name);
            match arg {
                crate::c_signature::CArg::TextPointer => {
                    args.push(format!("_cs_{var}.as_ptr().cast::<std::ffi::c_void>()"));
                }
                crate::c_signature::CArg::VectorPtr => {
                    args.push(format!("_vp_{var}.cast::<std::ffi::c_void>()"));
                }
                crate::c_signature::CArg::VectorPtrCount => {
                    args.push(format!("_vp_{var}.cast::<std::ffi::c_void>()"));
                    let count_ty = sig
                        .params
                        .get(c_i + 1)
                        .map_or("i64", crate::c_signature::CType::rust_type);
                    args.push(format!("(_vc_{var} as {count_ty})"));
                }
                crate::c_signature::CArg::Scalar => {
                    let ty = sig
                        .params
                        .get(c_i)
                        .map_or("i64", crate::c_signature::CType::rust_type);
                    args.push(format!("((var_{var}) as {ty})"));
                }
            }
            c_i += arg.slots();
        }
        let call = format!("unsafe {{ __c_{}({}) }}", sig.symbol, args.join(", "));

        // The return. `as i64` after a width-correct extern is the whole of the
        // fix the probe asked for.
        match (&sig.ret, def.returned().base()) {
            (crate::c_signature::CType::Void, _) => writeln!(w, "  {call};")?,
            (_, Type::Boolean) => writeln!(w, "  (({call}) != 0) as u8")?,
            // @PLN128 arc E — a float return. The extern already declares `f64`
            // / `f32` (`CType::rust_type`), so rustc gets the register class
            // right by construction and the cast only names loft's slot width.
            // The interpreter reaches the same value through its own rung
            // (`c_call::call_at_arity_f64`); this is the half that needs no
            // trampoline at all.
            (_, Type::Float) => writeln!(w, "  ({call}) as f64")?,
            (_, Type::Single) => writeln!(w, "  ({call}) as f32")?,
            // @PLN24 arc D — a `char *` return, copied into an owned `String`
            // (the wrapper is `-> String` via `returns_owned_string`).  The
            // three decisions are the interpreter's, restated: NULL is loft
            // null, the bytes end at the first NUL, invalid UTF-8 is replaced.
            // `c_call::c_text` is the twin — they are checked against each
            // other by the fixture matrix, which requires byte-identical output.
            (_, Type::Text(_)) => {
                writeln!(w, "  {{")?;
                writeln!(w, "    let _p = {call}.cast::<std::ffi::c_char>();")?;
                writeln!(
                    w,
                    "    if _p.is_null() {{ loft::state::STRING_NULL.to_string() }} else {{"
                )?;
                writeln!(
                    w,
                    "      String::from_utf8_lossy(unsafe {{ std::ffi::CStr::from_ptr(_p) }}.to_bytes()).into_owned()"
                )?;
                writeln!(w, "    }}")?;
                writeln!(w, "  }}")?;
            }
            _ => writeln!(w, "  ({call}) as i64")?,
        }
        writeln!(w, "}}")?;
        Ok(())
    }

    fn output_native_direct_call(
        &self,
        w: &mut dyn Write,
        d_nr: u32,
        qualified_symbol: &str,
    ) -> std::io::Result<()> {
        let def = self.data.def(d_nr);
        // @P321c browser-WASM fix (2026-05-29): for store-mutating `#native` fns
        // (those needing a `LoftStore` handle for a Reference arg or Vector/
        // Reference return), the `--html` lowering path cannot use the cdylib
        // ABI — `--html` produces a standalone wasm binary (no cdylib to
        // dlopen), and `loft::native_call` is gated behind
        // `feature = "native-extensions"` which is NOT enabled for wasm.
        // Without this guard, codegen emits the cdylib body (uses
        // `loft::native_call::enter`/`build_store`) alongside the extern
        // import the wasm-import-module preamble already declared, tripping
        // E0428 + E0433 + E0061 + E0308.
        //
        // Phase 1 (this guard): emit a graceful loft-aware stub matching the
        // fn's return type — `boolean` → `false`, `integer` → `0`, `text` →
        // empty `Str`, `reference` → null `DbRef`.  This matches loft's
        // existing semantics: `lib/imaging/src/imaging.loft::png()` already
        // null-checks `load_png` (returns null on `false`), and unmapped
        // file reads are observably the same.  Build succeeds; runtime is a
        // no-op.  NOT `unimplemented!()` (would trap any consumer the moment
        // they touch the fn).
        //
        // Routed via lib_plan-29: each library's `[wasm.bridge].routes`
        // map (populated into `data.wasm_bridge_routes`) names the
        // `pub fn` bridge in its per-library `wasm/src/lib.rs` crate.
        // The fallback below covers store-mutating `#native` fns with
        // no `[wasm.bridge]` declared.  Note: `state.replace_native`
        // does NOT work here — that mechanism is interpreter-only
        // (mutates `State::library`); `--html` generates a standalone
        // Rust binary that calls `n_load_png()` as a plain Rust
        // function with no `State` indirection at runtime.
        if self.wasm_browser {
            // #407 — the `[wasm.bridge].routes` table decides browser routing,
            // NOT the arg/return SHAPE.  A `text -> text` / `text -> boolean`
            // native (every crypto primitive) carries no struct/Reference arg
            // and no Vector/Reference return, yet still belongs to a bridge.
            // Consult the route table FIRST (the chokepoint), independent of
            // shape; only the shape-driven Phase-1 fallback below cares about
            // struct/ref shapes (those have no registered bridge to call).
            //
            // lib_plan-29 W1c (2026-05-29): the table is built from each
            // library's `[wasm.bridge]` manifest section
            // (`data.wasm_bridge_routes`) — no library symbols hard-coded in
            // the compiler crate.  Key is the `#native "symbol"`.
            let bridge_target =
                self.data
                    .wasm_bridge_routes
                    .get(def.native())
                    .map(|(bridge_crate, bridge_fn)| {
                        let crate_ident = bridge_crate.replace('-', "_");
                        format!("{crate_ident}::{bridge_fn}")
                    });
            if let Some(target) = bridge_target {
                return Self::output_wasm_bridge_call(w, def, &target);
            }
            // A NON-vector heap arg (Reference / data-enum / sorted / hash / index /
            // spatial) makes this a store-touching native with no host-import shape →
            // graceful stub.  D-html-vec: a `vector<T>` arg is EXCLUDED — it is a real
            // host import marshalled as `(ptr, count)` below (the pre-#423 behaviour the
            // GL upload/matrix natives rely on); #423 lumped it in here and stubbed the
            // upload calls, so nothing ever reached WebGL.
            let first_ref_arg = def.attributes.iter().find(|a| {
                !a.name.starts_with("__")
                    && a.typedef.base().heap_dep().is_some()
                    && !matches!(a.typedef.base(), Type::Vector(_, _))
            });
            let returns_loft_ref = matches!(
                def.returned().base(),
                Type::Vector(_, _) | Type::Reference(_, _)
            );
            if returns_loft_ref || first_ref_arg.is_some() {
                // Phase 1 fallback: graceful loft-aware stub for any
                // store-mutating #native fn that doesn't have a bridge
                // registered.  Matches loft semantics (`false`/null
                // return reads as "operation failed" not "trap").
                writeln!(w, "{{")?;
                writeln!(
                    w,
                    "  // @P321c browser-WASM Phase 1 stub: graceful no-op (no bridge registered for {})",
                    def.native()
                )?;
                match def.returned().base() {
                    Type::Void => {}
                    Type::Boolean => writeln!(w, "  0u8")?, // @PLN17: u8 storage form
                    Type::Integer(_) | Type::Float | Type::Single => writeln!(w, "  0")?,
                    Type::Text(_) => {
                        // @PLN10 N2 — cdylib text-native wrappers now return
                        // `-> String` (gated by `!def.native.is_empty()`); the
                        // graceful browser stub returns an empty owned String
                        // to match the flipped signature.
                        writeln!(w, "  String::new()")?;
                    }
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => {
                        writeln!(w, "  loft::keys::DbRef::NULL")?;
                    }
                    _ => {
                        writeln!(w, "  Default::default()")?;
                    }
                }
                writeln!(w, "}}")?;
                return Ok(());
            }
        }
        writeln!(w, "{{")?;
        writeln!(
            w,
            "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;

        // Pre-declare each `vector` arg's inner-record number before the call
        // expression.  A loft `vector` var is an OUTER record whose word at
        // `(rec, pos)` holds the inner vector record; `_vr_{var}` is that inner
        // record, which the call site passes as a `LoftRef` (mirroring the
        // interpreter's `ArgT::Vec` deref `rec = get_u32_raw(r.rec, r.pos)`).
        for attr in def.attributes() {
            if attr.name.starts_with("__") {
                continue;
            }
            if let Type::Vector(elem_tp, _) = attr.typedef.base() {
                let var = sanitize(&attr.name);
                writeln!(
                    w,
                    "  let _vr_{var} = loft::keys::store(&var_{var}, &stores.allocations).get_u32_raw(var_{var}.rec, var_{var}.pos);"
                )?;
                // D-html-vec: the browser host import takes the raw `(ptr, count)` of the
                // vector's element data — the JS glue reads it as `new Float32Array(mem,
                // ptr, count)`, NOT a LoftStore/LoftRef (the wasm binary has no cdylib
                // runtime).  Pre-declare the element count + a pointer into the store's
                // linear memory so the call arm can pass them (pre-#423 behaviour).
                if self.wasm_browser {
                    let elem = Self::vector_elem_rust_type(elem_tp);
                    writeln!(
                        w,
                        "  let _vc_{var} = if _vr_{var} == 0 {{ 0u32 }} else {{ loft::keys::store(&var_{var}, &stores.allocations).get_u32_raw(_vr_{var}, 4) }};"
                    )?;
                    writeln!(
                        w,
                        "  let _vp_{var}: *const {elem} = if _vr_{var} == 0 {{ std::ptr::null() }} else {{ loft::keys::store(&var_{var}, &stores.allocations).addr::<{elem}>(_vr_{var}, 8) as *const {elem} }};"
                    )?;
                }
            }
        }

        // @PLAN12 phase 3.5a (2026-05-24) — LoftStore forwarding for
        // store-allocating cdylib returns (Type::Vector / Type::Reference).
        // The cdylib needs a LoftStore handle to alloc the returned vector
        // / struct.  Construct one via the new `loft::native_call::build_store`
        // API and set up CURRENT_STORES for the cdylib's callbacks via the
        // RAII `enter` guard.  When no Reference/Vector arg is present
        // (random's n_rand_indices case), allocate against the null store
        // (stores.null()).  Type::Reference args + their DbRef→LoftRef
        // conversion (`to_loft_ref`) is a future extension for
        // imaging/graphics drains — not required for random.
        //
        // Bind the LoftStore via `transmute_copy` so rustc accepts it
        // at the cdylib call site even when there are TWO copies of
        // loft_ffi in the dependency graph (cdylib brings its own;
        // loft crate has its own).  Both are `#[repr(C)]` with
        // identical fields, so the bit-copy is safe.  Same trick the
        // text-return path uses for LoftStr (see comment ~30 lines
        // below in this function).
        // @P321c (2026-05-25) — a struct `Reference` ARG (not just a
        // Vector/Reference return) also needs a `LoftStore`: the cdylib reads
        // and/or writes the struct's fields through it (imaging's
        // `n_load_png(store, path, len, image)` decodes a PNG and writes
        // name/width/height + an allocated pixel vector into `image`).  The
        // store handle must point at the store the struct lives in — NOT the
        // null store — so the vector the cdylib allocates lands in the same
        // store as its owner (mirrors the interpreter's
        // `make_loft_store(stores, first_ref_store(args))` at
        // `src/extensions.rs:981`).
        // Any heap-typed arg (Reference / Vector / data-enum / sorted / hash /
        // index / spatial) pins the store and rides as a `LoftRef`, exactly as
        // the interpreter's `ref_arg_store` picks the first `LoftTag::Ref` arg
        // (a marshalled vector carries that tag too).  `heap_dep()` is the
        // canonical heap set; the outer DbRef's `.store_nr` is the store for
        // both Reference and Vector args.
        // D-html-vec: for the browser, a `vector` arg does NOT pin a LoftStore — it is
        // passed as a raw `(ptr, count)` pair (below), so it must not force the cdylib
        // `_ls` store-handle machinery.  Exclude it from the store-pin decision in the
        // wasm path only; the native cdylib path keeps #423's LoftRef convention.
        let first_ref_arg = def.attributes.iter().find(|a| {
            !a.name.starts_with("__")
                && a.typedef.base().heap_dep().is_some()
                && !(self.wasm_browser && matches!(a.typedef.base(), Type::Vector(_, _)))
        });
        // `returns_loft_ref` drives the RETURN conversion (`from_loft_ref`);
        // `needs_loft_store` drives the store-handle + guard + `_ls` first arg.
        // They diverge for a Reference-arg fn with a scalar return (imaging's
        // `load_png` returns `boolean`): store handle yes, return conversion no.
        let returns_loft_ref = matches!(
            def.returned().base(),
            Type::Vector(_, _) | Type::Reference(_, _)
        );
        let needs_loft_store = returns_loft_ref || first_ref_arg.is_some();
        if needs_loft_store {
            // Order matters: extract `store_nr` as a SEPARATE statement so it
            // doesn't dual-borrow `stores` alongside the build_store call
            // (rustc E0502).  A heap-typed arg pins the store to that arg's
            // store_nr; otherwise (vector-return only, e.g. random) the null
            // store hosts the freshly allocated return vector.
            if let Some(a) = first_ref_arg {
                let var = sanitize(&a.name);
                writeln!(w, "  let _store_nr = var_{var}.store_nr;")?;
            } else {
                writeln!(w, "  let _store_nr = stores.null().store_nr;")?;
            }
            writeln!(w, "  let _guard = loft::native_call::enter(stores);")?;
            writeln!(
                w,
                "  let _ls_src = loft::native_call::build_store(stores, _store_nr);"
            )?;
            // Reinterpret as the cdylib's LoftStore (same layout,
            // different crate identity).  Type of `_ls` inferred
            // from the call site below.
            writeln!(
                w,
                "  let _ls = unsafe {{ std::mem::transmute_copy(&_ls_src) }};"
            )?;
        }

        let needs_ret_cast = matches!(def.returned().base(), Type::Integer(_));
        // @PLN17: external Rust fns use `bool`; loft's boolean storage form is u8.
        // Wrap a boolean return `(call) as u8`; boolean args coerce `u8 -> bool` below.
        let needs_bool_ret = matches!(def.returned().base(), Type::Boolean);
        // P244 / @PLN10 N2: `text`-returning natives return `loft_ffi::LoftStr`
        // from the extern.  Capture it as a typed local, copy its bytes into an
        // OWNED `String`, and return that directly — the wrapper signature is
        // `-> String` (gated by `!def.native().is_empty()` at the header) and the
        // caller bridges `String` → `Str` via `Deref` (the @P304 path).  The
        // original P244 fix borrowed a `Str` from `stores.scratch`; N2 retired
        // the scratch hop (owned, freed at scope end — no program-lifetime leak).
        let needs_text_wrap = matches!(def.returned().base(), Type::Text(_));
        if needs_ret_cast || needs_bool_ret {
            write!(w, "  (unsafe {{ {qualified_symbol}(")?;
        } else if needs_text_wrap {
            // No type annotation: lib/server (and other native sub-crates)
            // bring their own copy of `loft_ffi`; binding by inferred type
            // sidesteps the "multiple versions of crate loft_ffi" rustc
            // E0308.  Both copies are structurally identical (`#[repr(C)]
            // pub struct LoftStr { pub ptr: *const u8, pub len: usize }`),
            // so the field reads on the next lines work either way.
            // Named `_ret_str` (not `_ls`) so it never collides with the
            // `_ls` LoftStore handle when a text-returning native also takes
            // a Reference arg (@P321c made that combination reachable).
            write!(w, "  let _ret_str = unsafe {{ {qualified_symbol}(")?;
        } else if returns_loft_ref {
            // Capture the LoftRef return; convert to DbRef after the call.
            write!(w, "  let _lr = unsafe {{ {qualified_symbol}(")?;
        } else {
            write!(w, "  unsafe {{ {qualified_symbol}(")?;
        }
        let mut first = true;
        if needs_loft_store {
            write!(w, "_ls")?;
            first = false;
        }
        for attr in def.attributes() {
            if attr.name.starts_with("__") {
                continue;
            }
            let var = sanitize(&attr.name);
            match attr.typedef.base() {
                Type::Text(_) => {
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(w, "var_{var}.as_ptr(), var_{var}.len()")?;
                }
                Type::Vector(_, _) if self.wasm_browser => {
                    // D-html-vec: the browser host import takes the raw `(ptr, count)`
                    // of the element data (`_vp_{var}` / `_vc_{var}` from the pre-declare
                    // block), matching the declared `ptr: *const T, count: u32` extern and
                    // the JS glue's `new Float32Array(mem, ptr, count)`.  This is the
                    // pre-#423 path #423 replaced with the LoftRef ABI below — which the
                    // wasm binary has no cdylib runtime to honour (→ the calls were stubbed
                    // out and Brick Buster rendered blank).
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(w, "_vp_{var}, _vc_{var}")?;
                }
                Type::Vector(_, _) => {
                    // A `vector` arg rides as a `LoftStore` + `LoftRef`, NOT a
                    // raw `(ptr, count)` pair — that mismatched the `#[loft_native]`
                    // bridge's `(LoftStore, LoftRef)` ABI and segfaulted.  The loft
                    // `vector` var is an OUTER record (a header whose word at
                    // `(rec, pos)` is the inner vector record); `_vr_{var}` (set up
                    // in the pre-declare block above) is that inner record, exactly
                    // what the interpreter's `ArgT::Vec` marshals
                    // (`extensions.rs` — `rec = get_u32_raw(r.rec, r.pos)`, pos 0).
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(
                        w,
                        "unsafe {{ std::mem::transmute_copy(&loft::codegen_runtime::to_loft_ref(loft::keys::DbRef {{ store_nr: var_{var}.store_nr, rec: _vr_{var}, pos: 0 }})) }}"
                    )?;
                }
                // Reference / data-enum / sorted / hash / index / spatial: a
                // heap-typed arg whose DbRef points DIRECTLY at the record
                // (no outer→inner deref, unlike Vector).  Matched via
                // `heap_dep()` so every keyed-collection kind shares the
                // @P321c LoftRef convention (the interpreter's `ArgT::Ref`,
                // which forwards `r.store_nr, r.rec, r.pos` verbatim).
                t if t.heap_dep().is_some() => {
                    // `var_{var}` is a DbRef pointing directly at the record
                    // (matching the interpreter's `ArgVal::Ref(r.store_nr, r.rec,
                    // r.pos)`).  `to_loft_ref` returns the loft crate's
                    // `loft_ffi::LoftRef`; `transmute_copy` reinterprets it as the
                    // cdylib's identically-`#[repr(C)]` copy without naming the
                    // type (avoids the "colliding StableCrateId" error when two
                    // `loft_ffi` copies are in the dep graph — same trick the
                    // `_ls` store handle uses above).
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(
                        w,
                        "unsafe {{ std::mem::transmute_copy(&loft::codegen_runtime::to_loft_ref(var_{var})) }}"
                    )?;
                }
                Type::Integer(_) | Type::Character => {
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(w, "var_{var} as _")?;
                }
                Type::Float => {
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(w, "var_{var}")?;
                }
                Type::Boolean => {
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    // @PLN17: loft holds the u8 storage form; the external Rust fn
                    // takes `bool` — coerce (255/0 -> false, 1 -> true).
                    write!(w, "var_{var} != 0")?;
                }
                _ => {
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(w, "var_{var}")?;
                }
            }
        }
        if needs_ret_cast {
            write!(w, ") }}) as i64")?;
        } else if needs_bool_ret {
            write!(w, ") }}) as u8")?; // @PLN17: external bool -> loft u8 storage form
        } else if needs_text_wrap {
            // @PLN10 N2 — cdylib `#native` text return: copy the foreign
            // `LoftStr` bytes into an OWNED `String` and return it directly.
            // The wrapper signature is `-> String` (gated by `!def.native
            // .is_empty()` at the header) and the caller bridges `String` →
            // `Str` via `Deref<Target=str>` (the @P304 path — same as the
            // curated `codegen_runtime` producers in Build 4).  No
            // `stores.scratch`: the `String` is owned by the caller and freed
            // at scope end instead of leaked into the program-lifetime buffer.
            writeln!(w, ") }};")?;
            writeln!(
                w,
                "  let _bytes: Vec<u8> = if _ret_str.ptr.is_null() {{ Vec::new() }} else {{ unsafe {{ std::slice::from_raw_parts(_ret_str.ptr, _ret_str.len) }}.to_vec() }};"
            )?;
            write!(w, "  unsafe {{ String::from_utf8_unchecked(_bytes) }}")?;
        } else if returns_loft_ref {
            // Convert cdylib's LoftRef return to a DbRef.  `_guard`
            // (Drop) clears CURRENT_STORES at function exit.  Transmute
            // is INLINED at the from_loft_ref call site — naming
            // `loft_ffi::LoftRef` as a type annotation in the
            // generated source triggers rustc's
            // "colliding StableCrateId" error when both the loft crate
            // and the cdylib pull loft_ffi.  Letting from_loft_ref's
            // parameter type drive the transmute_copy inference keeps
            // `loft_ffi` invisible to the generated code.
            writeln!(w, ") }};")?;
            write!(
                w,
                "  loft::codegen_runtime::from_loft_ref(stores, unsafe {{ std::mem::transmute_copy(&_lr) }})"
            )?;
        } else {
            write!(w, ") }}")?;
        }
        writeln!(w, "\n}}")
    }

    /// #407 — emit a browser-WASM wrapper body that routes a `#native` through
    /// its `[wasm.bridge].routes` entry: a call to `<crate>::<bridge_fn>` whose
    /// result the wrapper returns directly.
    ///
    /// The bridge `pub fn` runs in pure Rust inside the standalone `--html`
    /// wasm binary (no host import, no cdylib ABI), so its signature is the
    /// *loft-side* one: `stores: &mut Stores` first, then one argument per loft
    /// parameter in loft-side Rust types, returning a value of the loft-side
    /// return type.  Because the rest of codegen already chooses the wrapper's
    /// Rust signature from the loft return type (`returns_owned_string` →
    /// `-> String` for text, `rust_type(Result)` → `u8` for boolean, `i64` for
    /// integer), the bridge result needs only a per-type cast to land in that
    /// signature — NO store reshaping for a text/scalar return.  This is the
    /// clean convention: a `text -> text` native bridges with no per-fn
    /// Reference-out reshape.
    ///
    /// Argument ABI (mirrors the non-bridge path so a bridge fn reads naturally
    /// in pure Rust):
    /// - `text`   → `&str`     (by value — `var_x` is already `&str`)
    /// - `boolean`→ `bool`     (`var_x != 0`; loft holds the u8 storage form)
    /// - `integer`/`character` → coerced via `as _`
    /// - `float`/`single`      → by value
    /// - `Reference`/`Vector`  → `&DbRef` (the bridge works the store via
    ///   `stores`; matches the proven imaging-shape bridges)
    fn output_wasm_bridge_call(
        w: &mut dyn Write,
        def: &crate::data::Definition,
        target: &str,
    ) -> std::io::Result<()> {
        writeln!(w, "{{")?;
        writeln!(
            w,
            "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
        // The bridge result lands directly in the wrapper's return slot.  Match
        // the wrapper signature the rest of codegen chose for this return type.
        let needs_ret_cast = matches!(def.returned(), Type::Integer(_)); // wrapper -> i64
        let needs_bool_ret = matches!(def.returned().base(), Type::Boolean); // wrapper -> u8 (incl. boolean?)
        write!(w, "  ")?;
        write!(w, "{target}(stores")?;
        for attr in def.attributes() {
            if attr.name.starts_with("__") {
                continue;
            }
            let var = sanitize(&attr.name);
            match &attr.typedef {
                Type::Text(_) => write!(w, ", var_{var}")?,
                Type::Boolean => write!(w, ", var_{var} != 0")?,
                Type::Integer(_) | Type::Character => write!(w, ", var_{var} as _")?,
                Type::Float | Type::Single => write!(w, ", var_{var}")?,
                // Reference/Vector args: hand the bridge a `&DbRef`; it works the
                // store through the `stores` handle (the imaging-shape bridges).
                _ => write!(w, ", &var_{var}")?,
            }
        }
        write!(w, ")")?;
        if needs_ret_cast {
            writeln!(w, " as i64")?;
        } else if needs_bool_ret {
            // @PLN17: external `bool` -> loft `u8` storage form.
            writeln!(w, " as u8")?;
        } else {
            writeln!(w)?;
        }
        writeln!(w, "}}")
    }

    /// Map a loft vector element type to the Rust primitive type used for the
    /// raw-pointer calling convention.
    ///
    /// Native functions that accept `vector<T>` args receive a `*const ELEM_TYPE`
    /// pointer.  This function returns the Rust type name for each loft element type.
    fn vector_elem_rust_type(tp: &Type) -> &'static str {
        match tp {
            Type::Single => "f32",
            Type::Float => "f64",
            Type::Boolean => "u8",
            Type::Character => "u32",
            // @P310: the FFI data-pointer element width must match the
            // vector's STORAGE STRIDE, not its value range.  A plain
            // `vector<integer>` carries no `forced_size`, so it is stored at
            // the wide 8-byte stride (`byte_width() == 8`, `Type::size == 8`,
            // `vector_append` strides 8) — even though its bounds are the
            // signed-32 template (`is_wide() == false`).  Keying off
            // `vector_narrow_width()` (the same predicate the vector storage +
            // `OpGetVector`/`OpSetVector` use, `data.rs::vector_narrow_width`)
            // gives the pointer the storage-matching width: plain integer →
            // `i64` (matches the cdylib `vec<i64>` wrappers + `Canvas` pixel
            // semantics), narrow aliases keep their forced width.  (i8/i16
            // narrow vectors map to the unsigned same-width name — signedness
            // is moot, no `#native` FFI takes a narrow-int vector today.)
            Type::Integer(s) => match s.vector_narrow_width(false) {
                Some(1) => "u8",
                Some(2) => "i16",
                Some(4) => "i32",
                _ => "i64",
            },
            // Fallback for struct/enum elements: opaque bytes.
            _ => "u8",
        }
    }
}

fn emit_db_field(
    w: &mut dyn Write,
    struct_var: &str,
    field_name: &str,
    prefix: &str,
    builder: &str,
) -> std::io::Result<()> {
    let var = format!("{prefix}_{}", sanitize(field_name));
    writeln!(w, "    let {var} = {builder};")?;
    writeln!(w, "    db.field({struct_var}, \"{field_name}\", {var});")?;
    Ok(())
}

/// Render a compile-time `known_type` as a reference expression in the
/// generated `init()` body.  For real types (0..=u16::MAX-1) this is the
/// `t{N}` let-binding that `output_init` / `output_struct` emit at the
/// time the runtime id is assigned.  For the `u16::MAX` null sentinel
/// (used by `Type::Vector(Type::Unresolved, _)` etc.) we emit the raw
/// literal — there is no let-binding for it.
fn type_id_ref(known_type: u16) -> String {
    if known_type == u16::MAX {
        "u16::MAX".to_string()
    } else {
        format!("t{known_type}")
    }
}

/// Use this to register an enum in the runtime database.
/// Plain tag variants are registered with `u16::MAX`; struct-enum variants use the variant
/// struct's `known_type` so that `ShowDb` can dispatch to the variant's fields.
fn output_enum_values(
    w: &mut dyn Write,
    d_nr: u32,
    data: &Data,
    type_id: u16,
) -> std::io::Result<()> {
    let def = data.def(d_nr);
    let e_var = format!("t{type_id}");
    for a in def.attributes() {
        let variant_type = if matches!(a.typedef, Type::Enum(_, true, _)) {
            // Find the EnumValue definition whose parent is this enum and name matches.
            (0..data.definitions())
                .find(|&v| {
                    let v_def = data.def(v);
                    v_def.def_type == DefType::EnumValue
                        && v_def.parent == d_nr
                        && v_def.name == a.name
                })
                .map_or(u16::MAX, |v| data.def(v).known_type())
        } else {
            u16::MAX
        };
        if variant_type == u16::MAX {
            writeln!(w, "    db.value({e_var}, \"{}\", u16::MAX);", a.name)?;
        } else {
            writeln!(w, "    db.value({e_var}, \"{}\", t{variant_type});", a.name)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod p310_vector_elem_tests {
    use super::*;
    use crate::data::IntegerSpec;

    /// @P310 regression: the FFI element pointer width must track the vector's
    /// storage stride.  `vector_elem_rust_type` takes the ELEMENT type.  Plain
    /// `vector<integer>` (signed32 template, NOT `is_wide()`) is 8-byte-stride
    /// storage → must emit `i64`, not `i32`.
    #[test]
    fn vector_elem_rust_type_matches_storage_stride() {
        // Both plain-integer templates store at 8-byte stride → i64.
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::wide())),
            "i64"
        );
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::signed32())),
            "i64",
            "plain vector<integer> must emit *const i64 (storage is 8-byte stride) — @P310"
        );
        // Narrow aliases keep their forced storage width.
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::i32())),
            "i32"
        );
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::u16())),
            "i16"
        );
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::i16())),
            "i16"
        );
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::u8())),
            "u8"
        );
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::i8())),
            "u8"
        );
        // Non-integer element types unchanged.
        assert_eq!(Output::vector_elem_rust_type(&Type::Single), "f32");
        assert_eq!(Output::vector_elem_rust_type(&Type::Float), "f64");
    }
}

#[cfg(test)]
mod scrub_tests {
    use super::scrub_generated_crate_refs;

    #[test]
    fn rewrites_internal_module_refs_but_not_host_intrinsics() {
        let src = b"if !crate::rpc::print_or_capture(v) { print!(); }\n\
                    crate::store::Store::seal(s);\n\
                    crate::loft_host_print(p, l);\n\
                    crate::wasm::output_push(v);";
        let out = String::from_utf8(scrub_generated_crate_refs(src)).unwrap();
        // The two loft-internal modules are rewritten to `loft::`.
        assert!(out.contains("loft::rpc::print_or_capture"));
        assert!(out.contains("loft::store::Store::seal"));
        assert!(!out.contains("crate::rpc::"));
        assert!(!out.contains("crate::store::"));
        // Host-import intrinsics (the generated cdylib's OWN items) stay `crate::`.
        assert!(out.contains("crate::loft_host_print"));
        assert!(out.contains("crate::wasm::output_push"));
    }

    #[test]
    fn clean_source_is_unchanged() {
        let src = b"fn n_main(cell: &Cell) { loft::rpc::ok(); }";
        assert_eq!(scrub_generated_crate_refs(src), src.to_vec());
    }
}

#[cfg(test)]
mod p98_p34_tests {
    use super::Output;

    // @PLN98 P3.4 — the browser `loft_start` opt-in. A production client (default,
    // emit_live=false) ships a plain `Stores::new()` boot with NO live/debug tier;
    // a `--debug[=name]` client bootstraps the parked interpreter from the EMBEDDED
    // source and bakes the debug NAME the server uses to address it.
    #[test]
    fn wasm_start_gates_the_debug_tier_on_the_opt_in() {
        let p = crate::parser::Parser::new();
        let db = crate::database::Stores::new();
        let mut out = Output::new(&p.data, &db);
        out.wasm_browser = true;

        // Production (no --debug): plain boot, no debug tier, no embedded source.
        out.emit_live = false;
        let mut prod = Vec::new();
        out.emit_wasm_start(&mut prod).unwrap();
        let prod = String::from_utf8(prod).unwrap();
        assert!(
            prod.contains("Stores::new()"),
            "production boots plain: {prod}"
        );
        assert!(
            !prod.contains("bootstrap_from_bytes"),
            "no live bootstrap: {prod}"
        );
        assert!(
            !prod.contains("LOFT_DEBUG_NAME"),
            "no debug name baked: {prod}"
        );
        assert!(
            prod.contains("fn loft_start"),
            "still exports loft_start: {prod}"
        );

        // Debug client (`--debug=alice`): embedded bootstrap + the baked name + the
        // program source blob.
        out.emit_live = true;
        out.debug_name = Some("alice".to_string());
        out.program_src = Some("fn main() { print(\"hi\") }".to_string());
        out.live_fns = vec!["n_addup".to_string()];
        let mut dbg = Vec::new();
        out.emit_wasm_start(&mut dbg).unwrap();
        let dbg = String::from_utf8(dbg).unwrap();
        assert!(
            dbg.contains("static LOFT_DEBUG_NAME: &str = \"alice\""),
            "debug name baked for server addressing: {dbg}"
        );
        assert!(
            dbg.contains("bootstrap_from_bytes(LOFT_LIVE_FNS, LOFT_SRC)"),
            "boots the parked interpreter from embedded source: {dbg}"
        );
        assert!(
            dbg.contains("print(\\\"hi\\\")"),
            "the program source is embedded in LOFT_SRC: {dbg}"
        );
        assert!(
            dbg.contains("n_addup"),
            "the flippable fn table is emitted: {dbg}"
        );
    }
}
