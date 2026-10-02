// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-DispatchPublish` — the interpreter's lean dispatch loop no longer writes a crash
//! context per op: a crash report, a store-layer invariant report and the panic hook derive the
//! op from the position the loop stores for the allocator and from the bytecode, both
//! registered once per loop.  What that can break is attribution: a fault named against the
//! wrong op or the wrong position, or a report after the loop naming nothing.  So a native the
//! program calls asks, WHILE the loop runs it, what op this thread is on — it must be the
//! native call itself, at two different positions for two calls — and after the run the last
//! op must still be named.
use std::cell::RefCell;

use loft::compile;
use loft::database::Stores;
use loft::keys::DbRef;
use loft::parser::Parser;
use loft::state::State;

thread_local! {
    static SEEN: RefCell<Vec<(u32, u16, u32, &'static str)>> = const { RefCell::new(Vec::new()) };
}

fn n_probe_ctx(_stores: &mut Stores, _stack: &mut DbRef) {
    let (pc, op, fn_d_nr) = loft::crash_report::last_context();
    let name = loft::crash_report::last_op_name();
    SEEN.with(|s| s.borrow_mut().push((pc, op, fn_d_nr, name)));
}

const SRC: &str = r#"
fn probe_ctx();

fn main() {
  probe_ctx();
  x = 0;
  for i in 0..3 { x += i; }
  probe_ctx();
  println("x={x}");
}
"#;

#[test]
fn a_running_lean_loop_names_the_op_it_is_on() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse_str(SRC, "<publish-probe>", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "probe source must parse clean: {:?}",
        p.diagnostics.lines()
    );
    loft::scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database.clone());
    state.static_fn("n_probe_ctx", n_probe_ctx);
    compile::byte_code(&mut state, &mut p.data);
    SEEN.with(|s| s.borrow_mut().clear());
    state.execute_argv("main", &p.data, &[]);

    let seen = SEEN.with(|s| s.borrow().clone());
    assert_eq!(seen.len(), 2, "both probes ran: {seen:?}");
    for (pc, op, fn_d_nr, name) in &seen {
        assert_eq!(
            *name, "OpStaticCall",
            "the op named while a native runs is the native call: {seen:?}"
        );
        assert_ne!(*pc, u32::MAX, "a position is published: {seen:?}");
        assert_eq!(
            *fn_d_nr,
            u32::MAX,
            "the lean loop does not track the function"
        );
        assert_eq!(*op, seen[0].1, "one opcode for both calls: {seen:?}");
    }
    assert_ne!(
        seen[0].0, seen[1].0,
        "two calls at two positions — not a context frozen at the first: {seen:?}"
    );
    // After the loop: the last op dispatched is still named, not an empty context.
    let (pc, _op, _fd) = loft::crash_report::last_context();
    assert!(
        pc != u32::MAX && pc > seen[1].0,
        "after the run the last op is named, past the second probe ({}): pc {pc}",
        seen[1].0
    );
}
