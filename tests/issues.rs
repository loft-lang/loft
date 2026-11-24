// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Minimal reproducing tests for known open issues in the loft runtime.
//! Each test isolates exactly the bug pattern described in doc/claude/PROBLEMS.md.
//! Broken tests are marked #[ignore] so they are tracked but do not break CI.
//!
//! The tests live in `tests/issues/`, one part per subject; this file keeps the imports and the
//! helpers more than one part uses.  Each part's header says what it holds:
//! `runtime_basics`, `fields_and_lambdas`, `json_values`, `loop_returns`, `inconsistencies`,
//! `json_diagnostics`, `json_reading`, `enhancements`, `locals_and_coroutines`, `json_writing`,
//! `modules`, `sweep_findings`, `references`, `entry_returns` and `reachability`.

extern crate loft;

mod testing;

use loft::compile::byte_code;
use loft::data::Value;
use loft::file_access as fa;
use loft::logger::{Logger, RuntimeLogConfig};
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;
use std::sync::{Arc, Mutex};

// ── T1-1: Non-zero exit code on runtime error (production mode) ───────────────
// In normal mode a failing assert/panic aborts via Rust panic!().
// In production mode (--production flag) the error is logged and execution
// continues — main.rs must exit(1) via had_fatal.  These tests verify that
// `Stores::had_fatal` is set correctly so the binary-level exit code is right.

// Helper: compile loft code and return a State ready for execution.
fn compile_for_production(code: &str) -> (State, loft::data::Data) {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(code, "t1_1_test", false);
    assert!(
        p.diagnostics.lines().is_empty(),
        "Parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    (state, p.data)
}

// Attach a production-mode logger (writes to /dev/null) to a State.
fn attach_production_logger(state: &mut State) {
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    let lg = Logger::new(config, None);
    state.database.logger = Some(Arc::new(Mutex::new(lg)));
}

#[path = "issues/enhancements.rs"]
mod enhancements;
#[path = "issues/entry_returns.rs"]
mod entry_returns;
#[path = "issues/fields_and_lambdas.rs"]
mod fields_and_lambdas;
#[path = "issues/inconsistencies.rs"]
mod inconsistencies;
#[path = "issues/json_diagnostics.rs"]
mod json_diagnostics;
#[path = "issues/json_reading.rs"]
mod json_reading;
#[path = "issues/json_values.rs"]
mod json_values;
#[path = "issues/json_writing.rs"]
mod json_writing;
#[path = "issues/locals_and_coroutines.rs"]
mod locals_and_coroutines;
#[path = "issues/loop_returns.rs"]
mod loop_returns;
#[path = "issues/modules.rs"]
mod modules;
#[path = "issues/reachability.rs"]
mod reachability;
#[path = "issues/references.rs"]
mod references;
#[path = "issues/runtime_basics.rs"]
mod runtime_basics;
#[path = "issues/sweep_findings.rs"]
mod sweep_findings;
