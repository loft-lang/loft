// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Forward references across the two passes, `use` namespaces per library, multi-file packages,
//! and hooks declared inside a library.

use super::*;

// ── pass-1 index of a vector whose element struct is forward-referenced ──────
// `f` indexes `vector<Later>` before `struct Later` is registered.  In pass-1
// the element type is still `Unknown`, so `parse_vector_index`'s
// `def(type_elm(etp))` saw `u32::MAX` and panicked ("Unknown definition").  The
// fix substitutes the builtin `reference` def for the unresolved element so a
// placeholder read builds; pass-2 (every def registered) resolves `Later` and
// rebuilds the real read.  A genuinely-undefined element still errors at the
// type declaration, so this never masks a real typo.  The `42` proves pass-2
// produced a correct read, not just a non-crashing parse.
#[test]
fn forward_ref_struct_vector_index() {
    code!(
        "fn f(v: vector<Later>) -> integer { v[0].x }
struct Later { x: integer }
fn run() -> integer { v = [Later { x: 42 }]; f(v) }"
    )
    .expr("run()")
    .result(Value::Int(42));
}

// ── forward-reference resolution: the two-pass invariant ────────────────────
// loft's parser runs pass-1 (register definitions) then pass-2 (resolve bodies
// with everything registered), and pass-2 only runs if pass-1 emitted zero
// errors.  A type referenced before its definition is legitimately `Unknown` in
// pass-1 and resolves in pass-2 — so pass-1 must DEFER on it (no hard error, no
// panic, no token desync) or it deletes the very pass that would resolve it.
// These guard three positions that each used to break pass-1 differently; the
// values prove pass-2 produced a correct read, not just a non-crashing parse.

// Return type named before its struct definition.  A forward `-> Cell` registers
// an `Unknown` stub, so the body's `Cell { … }` construction found a non-struct
// def and desynced into "Expect token ;".  Fixed by treating the stub as
// not-yet-real in the construction dispatch (defer; pass-2 sees the real struct).
#[test]
fn forward_ref_return_type() {
    code!(
        "fn make2() -> Cell { Cell { n: 2 } }
struct Cell { n: integer }
fn run() -> integer { make2().n }"
    )
    .expr("run()")
    .result(Value::Int(2));
}

// Local type annotation named before its struct.  Same `Unknown`-stub
// construction desync as the return-type case, via `c: Cell = Cell { … }`.
#[test]
fn forward_ref_local_annotation() {
    code!(
        "fn use3() -> integer { c: Cell = Cell { n: 3 }; c.n }
struct Cell { n: integer }
fn run() -> integer { use3() }"
    )
    .expr("run()")
    .result(Value::Int(3));
}

// Direct struct field whose type is defined later.  `b.inner` reads a field
// whose declared type is still `Unknown` in pass-1; `get_val` emitted "Field
// access not supported on type unknown", killing pass-2 before
// `actual_types_deferred` resolved the field type.  Fixed by deferring the
// field read on an `Unknown` type in pass-1.
#[test]
fn forward_ref_struct_field_type() {
    code!(
        "struct Box { inner: Cell }
struct Cell { n: integer }
fn run() -> integer { b = Box { inner: Cell { n: 4 } }; b.inner.n }"
    )
    .expr("run()")
    .result(Value::Int(4));
}

// ── @P375 — pass-1 must DEFER, not break, on three more forward positions ────
// A dependency imported at a high source number is parsed AFTER the importing
// package on pass 1 (the package loader reserves source slots eagerly but parses
// bodies in todo-stack order), so a cross-package type is legitimately Unknown
// while the dependent's body parses.  The same shape reproduces single-file with
// a plain forward reference.  Each test below broke pass 1 a different way before
// the fix and now resolves on pass 2; the value proves a correct read, not just a
// non-crashing parse.  The full cross-package boundary matrix lives in the @P375
// investigation probes.

// For-loop over a forward-referenced `vector<struct>` field.  `for it in m.items`
// makes `m.items` Unknown in pass 1, so `for_type` fell to its catch-all and
// returned `Type::Null`; the loop body's `it.n` then hit `field()` on a Null type
// (past the `Type::Unknown` defer-guard) and hard-errored "Unknown type null",
// aborting pass 1 before the type could resolve.  Fixed by returning `Unknown`
// (not `Null`) so the read routes through the existing defer-guard.
#[test]
fn forward_ref_for_loop_vector_struct_field() {
    code!(
        "fn build(m: Map) -> integer { s = 0; for it in m.items { s = s + it.n; } s }
struct Inner { n: integer }
struct Map { items: vector<Inner> }
fn run() -> integer { build(Map { items: [Inner { n: 5 }] }) }"
    )
    .expr("run()")
    .result(Value::Int(5));
}

// `match` on a forward-referenced enum.  With the subject enum still an Unknown
// stub in pass 1, every arm hit the `bad_variant` skip path — which consumed the
// `=> expr` body but NOT the trailing comma, so the next iteration saw the leading
// `,` instead of a variant name, broke early, and desynced into "Expect token }".
// Fixed by consuming the optional trailing comma in the skip path.
#[test]
fn forward_ref_enum_match() {
    code!(
        "fn pick() -> integer { c = Color::Green; match c { Color::Red => 0, Color::Green => 6 } }
enum Color { Red, Green }
fn run() -> integer { pick() }"
    )
    .expr("run()")
    .result(Value::Int(6));
}

// Local `vector<struct>` literal of a forward-referenced element (@P373).  In
// pass 1 the element is Unknown, so the real `main_vector<Inner>` wrapper is
// born during pass-2 body codegen — AFTER this file's `fill_all` registration
// sweep — and reaches codegen with no database `known_type` AND no laid-out
// field positions.  Two faults followed: codegen baked `OpDatabase(db_tp=
// u16::MAX)` (panic in `set_default_value`), and even once that was registered,
// the wrapper's `vector` field sat at position `u16::MAX`, so `OpGetField` read
// through a bogus offset and corrupted the interpreter free path — a heap write
// that SIGSEGV'd at scope exit AFTER the correct value printed.  Fixed by
// `vector_wrapper_known_type` (vectors.rs): register the wrapper on the spot
// then `database.finish()` it so the field positions are laid out before
// codegen consumes them.  The `result(8)` proves correctness; soundness (no
// teardown SIGSEGV) is what regressed, so this also doubles as a free-path
// guard on both backends.
#[test]
fn forward_ref_local_vector_literal() {
    code!(
        "fn pull() -> integer { v = [Cell { n: 8 }]; v[0].n }
struct Cell { n: integer }
fn run() -> integer { pull() }"
    )
    .expr("run()")
    .result(Value::Int(8));
}

// Struct with an INLINE field of a forward-referenced struct (`inner: Cell`,
// @P373).  `Box` is laid out (pass-1 `fill_database`) before `Cell`, and the
// embedded-reference arm read `Cell`'s `known_type` directly — still u16::MAX —
// instead of laying `Cell` out first (as the vector / tuple arms do).  `Box.inner`
// then landed at offset u16::MAX, never repaired on pass 2 (`finish_type` skips an
// already-sized type), so `b.inner.n` read through a bogus offset and corrupted
// the free path: a non-deterministic SIGSEGV at scope exit AFTER the correct
// value printed.  Fixed by recursing into the inline content first in both the
// interpreter (`fill_database`) and the native db-init generator.  A `result`
// proves correctness; soundness (no teardown crash) is the real guard, so these
// run their full free path.  This is the inline-struct sibling of @P373.
#[test]
fn forward_ref_inline_struct_field() {
    code!(
        "struct Box { inner: Cell }
struct Cell { n: integer }
fn run() -> integer { b = Box { inner: Cell { n: 4 } }; b.inner.n }"
    )
    .expr("run()")
    .result(Value::Int(4));
}

// Two-level inline forward reference (`Outer.mid: Mid`, `Mid.inner: Cell`), both
// hosts declared before their content — exercises the recursive content-layout
// transitively (Outer → Mid → Cell).
#[test]
fn forward_ref_nested_inline_struct() {
    code!(
        "struct Outer { mid: Mid }
struct Mid { inner: Cell }
struct Cell { n: integer }
fn run() -> integer { o = Outer { mid: Mid { inner: Cell { n: 9 } } }; o.mid.inner.n }"
    )
    .expr("run()")
    .result(Value::Int(9));
}

// Enum struct-variant with an inline forward-referenced field (`Sq { side: Cell }`
// before `struct Cell`) — the EnumValue layout path takes the same inline-content
// arm, so it had the same offset-u16::MAX corruption.
#[test]
fn forward_ref_enum_variant_inline_field() {
    code!(
        "enum Shape { Sq { side: Cell } }
struct Cell { n: integer }
fn run() -> integer { s = Sq { side: Cell { n: 2 } }; match s { Sq { side } => side.n } }"
    )
    .expr("run()")
    .result(Value::Int(2));
}

// ── @P379 — `use` namespaces struct types per library ────────────────────────
// Two libraries each defining `struct Chunk` with DIFFERENT field layouts
// (moros_map's holds vector<Hex>, hex_world's holds vector<Cell>) must load
// together without the `Double structure type Chunk` internal panic, and each
// library's Chunk-bearing collections must resolve to the CORRECT per-library
// content type.  Before the fix, `use hex_world; use moros_map;` panicked at
// src/database/types.rs:53.
#[test]
fn p379_two_libs_same_struct_name() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    // hex_world was extracted to loft-libs-world 2026-06-01 (W.2); the
    // fixture lives under tests/fixtures/libs/ from Phase 6.12 onwards.
    // moros_map still lives in `lib/`.
    p.lib_dirs.push("tests/fixtures/libs".to_string());
    p.lib_dirs.push("lib".to_string());
    p.parse("tests/multilib/p379_lib_namespace.loft", false);
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(
        errors.is_empty(),
        "parse errors loading two libs: {errors:?}"
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    // production logger so an in-loft assert failure sets had_fatal instead of
    // aborting the test binary.
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    state.database.logger = Some(Arc::new(Mutex::new(Logger::new(config, None))));
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "per-library Chunk fields resolved incorrectly (an in-loft assert failed)"
    );
}

// ── use-region fixpoint — manifest dep that is a MULTI-FILE package ──────────
// `mfdep_app` declares `mfdep_leaf` only as a manifest `[dependencies]` edge
// (no source `use`), so the multi-file `mfdep_leaf` reaches the lexer through
// the pending-deps loop rather than the explicit-`use` pre-scan.  Its entry
// file opens with `use mfdep_leafmod;`.  Before parse_file wrapped the
// pre-scan + pending-deps drain in a fixpoint loop, the pending loop parked the
// lexer on that un-pre-scanned entry file, and the main definition-loop then
// read its legitimately-leading `use` as "use statements must appear before all
// definitions" — and never imported it.  This guards that a multi-file package
// pulled purely via a manifest edge still has its leading uses pre-scanned.
#[test]
fn manifest_dep_multifile_use_order() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.lib_dirs.push("tests/fixtures/libs".to_string());
    p.parse("tests/multilib/mfdep_use_order.loft", false);
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(
        errors.is_empty(),
        "multi-file manifest dep tripped the use-region check: {errors:?}"
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    state.database.logger = Some(Arc::new(Mutex::new(Logger::new(config, None))));
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "manifest-dep multi-file package failed to load/resolve at runtime"
    );
}

// ── loft#797 — a field whose type a LATER module declares still gets storage ─
//
// `fwd797`'s entry `use`s `fwd797_inner` and only then declares the structs that
// module's fields name, so `fwd797_inner` is laid out while every one of those
// types is still a forward-reference stub.  `fill_database` silently skips a
// field it cannot size and nothing revisits an already-registered type, so the
// declaration and the layout disagreed from then on: `position()` answered
// `u16::MAX` for the missing field and that flowed out as the field OFFSET, so
// writes landed at `record + 65535` — in the neighbouring records' bytes.
//
// The fixture covers each spelling that reaches the layout differently (inline
// struct, `vector<T>`, a nullable `T?`, and a host whose own field is one of the
// structs that had to wait) and the in-loft `main` checks the SIZES as well as
// the values: a read follows whatever offsets the layout ended up with, so
// reading a field back cannot by itself prove the field has storage.
#[test]
fn forward_module_type_gets_a_slot() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.lib_dirs.push("tests/fixtures/libs".to_string());
    p.parse("tests/multilib/fwd797_layout.loft", false);
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(
        errors.is_empty(),
        "a later-declared field type failed to compile: {errors:?}"
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    state.database.logger = Some(Arc::new(Mutex::new(Logger::new(config, None))));
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "a field or a size assertion in fwd797_layout.loft failed"
    );
}

// ── loft#801 — a module may name the entry's type in an EXPRESSION, not just ─
// ── in a declaration ─────────────────────────────────────────────────────────
//
// `fwd801`'s entry `use`s `fwd801_inner` and only then declares the types that
// module names, so every mention there is a forward reference to a file suspended
// further up the `use` chain.  Whether it resolved used to depend on the SPELLING:
// a written type went through `parse_type`, which leaves a `DefType::Unknown` stub
// for the entry's declaration to adopt in place, and an expression left nothing to
// adopt.  So `r: F801Roofs = F801Roofs { … }` compiled and the identical
// `r = F801Roofs { … }` did not.
//
// The fixture covers each spelling that reaches the name differently — construction
// alone, a vector literal, iterating that vector, the type as a value argument, and
// a typedef (`parse_typedef` reported the waiting stub as a name clash where
// `parse_struct` and `parse_enum` both adopt it).
#[test]
fn module_names_the_entry_type_in_an_expression() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.lib_dirs.push("tests/fixtures/libs".to_string());
    p.parse("tests/multilib/fwd801_body.loft", false);
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(
        errors.is_empty(),
        "a type named only in an expression failed to resolve: {errors:?}"
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    state.database.logger = Some(Arc::new(Mutex::new(Logger::new(config, None))));
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "an assertion in fwd801_body.loft failed"
    );
}

// ── @PLN125 arc B — a scope-end hook declared inside a LIBRARY runs ──────────
//
// The hook belongs to its type, and a library's symbols are module-scoped
// (@PLN102 C97).  Both askers — the emitter that puts the drop call in, and the
// never-read lint that stays quiet for a binding held only for its drop — run
// AFTER parsing, when the current source is the main program, so their `def_nr`
// resolved only the hooks @PLN102 C97 had also injected into the global
// namespace: the `pub` ones.  A private `OpDrop` was accepted by
// `check_drop_signature` and then never called anywhere, including inside its
// own package.
//
// That is a silent resource leak, and it is the exact shape @PLN138 gives every
// SQL backend: a cursor owning a `sqlite3_stmt *`, released at the closing
// brace.  Nothing calls `OpDrop` by name, so a library has every reason to keep
// it private — which is why the private case is the one the fixture declares.
//
// A SUBPROCESS on both backends rather than the in-process parser idiom its
// neighbours use, because a drop's only observable is I/O: the hook receives
// only `self` and a struct field copies at construction, so it cannot write back
// into the script's own data.  Each leg gets its own trace file — two legs
// sharing one would interleave their appends into a third program neither ran.
#[test]
fn a_private_scope_end_hook_in_a_library_runs() -> std::io::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for backend in ["--interpret", "--native"] {
        let trace = std::env::temp_dir().join(format!(
            "loft_dropscope{}.tmp",
            backend.trim_start_matches('-')
        ));
        let _ = fa::remove_file(&trace);
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(root.join("tests/fixtures/libs"))
            .arg(root.join("tests/multilib/drop_scope_hook.loft"))
            .env("LOFT_DROPSCOPE_TRACE", &trace)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success() && stdout.contains("drop scope hook ok"),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        // The script asserts each trace itself; this catches the failure mode a
        // script cannot see, where the hook fires from a SECOND mechanism and
        // the runtime reports the extra free rather than the script reporting a
        // wrong trace.
        assert!(
            !stdout.contains("BUG (#"),
            "{backend}: a drop must not provoke an internal fault:\n{stdout}"
        );
        let _ = fa::remove_file(&trace);
    }
    Ok(())
}

// ── loft#847 — an associated type binds as a TYPE, not as a place ───────────
//
// An implementor's return type carries a dep list indexed in its OWN frame, and
// it is non-empty exactly when the returned record comes from a NESTED CALL
// rather than an inline construction.  The companion binding recorded that type
// verbatim, so a monomorph substituted those indices into the CALLER, where the
// same numbers name unrelated locals: the caller's binding became a view of
// whatever variable 1 happened to be, and the free landed on a stack record.
//
// **The value stays right, which is why this needs its own reader.**  The `#306`
// guard REFUSES the wrong free rather than performing it, so every assertion in
// the script passed while the ownership underneath was broken — and `#306` is an
// `eprintln`, which the in-process script harness never looks at.  A subprocess
// is the only thing that can see it.
//
// Both axes are in the fixture: the delegating producer AND the associated-type
// bound.  Its concrete-bound control delegates identically and was always clean.
#[test]
fn a_delegating_producer_binds_its_companion_cleanly() -> std::io::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg(root.join("tests/scripts/pln125-a2c-companion.loft"))
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success() && stdout.contains("pln125 a2c companion ok"),
            "{backend} exited {}: stdout={stdout:?} stderr={stderr:?}",
            out.status,
        );
        assert!(
            !stderr.contains("BUG (#"),
            "{backend}: a companion must not carry the implementor's deps into the \
             monomorph:\n{stderr}"
        );
    }
    Ok(())
}
