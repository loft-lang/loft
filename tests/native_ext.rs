// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A7.1 — `#native "symbol_name"` annotation tests.
//!
//! Tests that loft functions annotated with `#native "symbol_name"` dispatch to
//! a Rust function registered via `State::register_native()`.

extern crate loft;

use loft::compile::byte_code;
use loft::database::Stores;
use loft::keys::DbRef;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;

mod common;
use common::cached_default;

fn n_double_it(stores: &mut Stores, stack: &mut DbRef) {
    let v = stores.get::<i32>(stack);
    stores.put(stack, v * 2);
}

fn n_add_two(stores: &mut Stores, stack: &mut DbRef) {
    let v = stores.get::<i32>(stack);
    stores.put(stack, v + 2);
}

/// A7.1: basic integer native function registered and called from loft.
/// Uses a BARE `#native` — the symbol defaults to the fn name (`n_double_it`).
/// (@PLAN12: an explicit `#native "n_double_it"` here is now a parse error
/// because the string equals the default; the override path — a symbol that
/// DIFFERS from the fn name — is covered separately below.)
#[test]
fn native_integer_function() {
    let native_decl = r#"
pub fn double_it(x: integer) -> integer;
#native
"#;
    let source = r#"
fn main() {
    println("{double_it(21)}")
}
"#;
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    assert!(
        p.diagnostics.is_empty(),
        "diagnostics: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    state.register_native("n_double_it", n_double_it);
    byte_code(&mut state, &mut p.data);
    state.execute_argv("main", &p.data, &[]);
}

/// A7.1: symbol name differs from the loft function name.
/// `say_hi` dispatches to `n_add_two` — verifying that the `#native` symbol
/// name overrides the default `n_<fn>` name lookup.
#[test]
fn native_symbol_name_differs_from_fn_name() {
    let native_decl = r#"
pub fn say_hi(x: integer) -> integer;
#native "n_add_two"
"#;
    let source = r#"
fn main() {
    println("{say_hi(40)}")
}
"#;
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    assert!(
        p.diagnostics.is_empty(),
        "diagnostics: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    state.register_native("n_add_two", n_add_two);
    byte_code(&mut state, &mut p.data);
    state.execute_argv("main", &p.data, &[]);
}
