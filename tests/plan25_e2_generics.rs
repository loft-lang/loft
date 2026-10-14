// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN25 E2 — generic functions over a nullable `vector<T>`.  Two fixes are
//! pinned here, both load-bearing for E2 shipping default-on:
//!
//! 1. **The generic parameter stays dense.**  `e2_nullable_elem` must NOT rewrite
//!    `vector<T>` (T = the opaque type-variable stub) to `vector<__nullable<T>>`.
//!    If it does, T is buried inside the synthetic enum and the "T appears in the
//!    first parameter" check + the `-> T` return unification both fail.  Verified
//!    by reading the generic template's first-param type: it must be
//!    `Vector(Reference(T))`, not `Vector(Enum(__nullable<…>))`.
//!
//! 2. **Monomorph names are valid Rust identifiers.**  Instantiating a generic
//!    over a synthetic element type whose NAME carries angle brackets
//!    (`__nullable<Row>`) produced a def named `t_15__nullable<Row>_count`, which
//!    native codegen emits verbatim as a Rust fn header — rustc then reads `<Row>`
//!    as a chained comparison (`error: comparison operators cannot be chained`).
//!    The mangle site now flattens `<>,` to `_`.  Verified by scanning EVERY
//!    `t_`-prefixed definition for bracket chars.
//!
//! Single `#[test]` so the process-global `LOFT_E2_SYNTH` gate is set with no
//! concurrent parsing in this binary (same rationale as `plan25_e2_layout.rs`).

extern crate loft;

use loft::data::Type;
use loft::file_access as fa;
use loft::parser::Parser;

const SRC: &str = r#"
struct Row { id: integer, tag: text }

// `first` returns T; `count` returns a non-T (so T monomorphises to the inline
// `__nullable<Row>` element type — the bracket-name case that broke native).
fn first<T>(v: vector<T>) -> T { v[0] }
fn count<T>(v: vector<T>) -> integer { c = 0; for x in v { c += 1; } c }

fn test() {
  v: vector<Row> = [Row{id:7, tag:"a"}, Row{id:8, tag:"b"}];
  r = first(v);
  n = count(v);
}
"#;

#[test]
fn generic_vector_param_stays_dense_and_monomorph_names_are_identifier_safe() {
    // SAFETY: this binary holds a single test, so no other thread parses while
    // the gate is set (Edition-2024 `set_var` is `unsafe` for the
    // multi-threaded-mutation hazard we avoid by construction here).
    unsafe {
        std::env::set_var("LOFT_E2_SYNTH", "1");
    }
    // Parse a real FILE (source = MAIN_SOURCE); `parse_str` leaves source at
    // STD_SOURCE, which the E2 scaffolding pass skips.
    let dir = std::env::temp_dir().join("loft_e2_generics_probe");
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join("e2_generics.loft");
    fa::write(&path, SRC).expect("write probe");
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse(&path.to_string_lossy(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "generics-over-nullable must parse clean (synth on): {:?}",
        p.diagnostics.lines()
    );

    // (1) The generic template's first parameter is still `vector<T>` (dense):
    // the element is `Reference(T-stub)`, NOT the synthetic nullable enum.
    // The template is stored under a mangled name, so locate it by kind + the
    // user-facing original name rather than a bare `def_nr("first")`.
    // User functions are stored as `n_<name>`; the generic template keeps that
    // name (its def-type is `Generic`, not `Function`).
    let first_d = p.data.def_nr("n_first");
    assert_ne!(first_d, u32::MAX, "generic template `n_first` is defined");
    assert_eq!(
        p.data.def_type(first_d),
        loft::data::DefType::Generic,
        "`n_first` is the generic template"
    );
    let param0 = p.data.def(first_d).attributes()[0].typedef.clone();
    let Type::Vector(elem, _) = &param0 else {
        panic!("first's param must be a vector, got {param0:?}");
    };
    match elem.as_ref() {
        Type::Reference(_, _) => {} // dense type-var element — correct
        Type::Enum(syn, _, _) => panic!(
            "generic vector<T> was wrongly rewritten to the nullable enum `{}` — \
             the type-var skip in e2_nullable_elem regressed",
            p.data.def(*syn).name
        ),
        other => panic!("unexpected generic element type {other:?}"),
    }

    // (2) No instantiated method name carries angle brackets — every `t_…`
    // definition must be a valid Rust identifier for native codegen.
    let mut offenders: Vec<String> = Vec::new();
    for d in 0..p.data.definitions() {
        let name = p.data.def(d).name();
        if name.starts_with("t_")
            && (name.contains('<') || name.contains('>') || name.contains(','))
        {
            offenders.push(name.to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "monomorph names must be identifier-safe (native codegen emits them as \
         Rust fn headers); found bracketed names: {offenders:?}"
    );

    unsafe {
        std::env::remove_var("LOFT_E2_SYNTH");
    }
}
