// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN63 extract-function ([EXTRACT.md]) — the intra-function data-flow engine.
//! E1: map a line selection to the enclosing function's contiguous TOP-LEVEL
//! statement slice, refusing anything that does not map to whole statements.

use loft::file_access as fa;
use loft::lsp::{ExtractEdit, extract_function, extract_inputs, extract_outputs, extract_range};

/// Apply an `ExtractEdit`: replace the selection lines with the call, then append
/// the new function after the buffer (as the LSP `WorkspaceEdit` does).
fn apply(text: &str, e: &ExtractEdit) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let (lo, hi) = ((e.start_line - 1) as usize, (e.end_line - 1) as usize);
    let mut out: Vec<String> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if i == lo {
            out.push(e.call.clone());
        } else if i > lo && i <= hi {
            // skip — replaced by the call
        } else {
            out.push((*l).to_string());
        }
    }
    format!("{}\n{}\n", out.join("\n"), e.new_function)
}

/// The program's stdout on the interpreter (for behaviour-preserving checks).
fn run(src: &str) -> String {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    fa::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("extract_run_{}.loft", src.len()));
    fa::write(&file, src).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg(&file)
        .env("LOFT_TIMEOUT", "30")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run loft");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// 1  fn f() -> integer {
// 2    a = 1;
// 3    b = a + 2;
// 4    c = a + b;
// 5    return c;
// 6  }
const F: &str = "fn f() -> integer {\n  a = 1;\n  b = a + 2;\n  c = a + b;\n  return c;\n}\n";

#[test]
fn e1_selection_maps_to_a_top_level_statement_slice() {
    // A single statement (line 2) → a slice; more statements → a wider op range.
    let one = extract_range(F, "buf.loft", "default", 2, 2).expect("line 2 = `a = 1`");
    let three = extract_range(F, "buf.loft", "default", 2, 4).expect("lines 2-4 = a, b, c");
    assert_eq!(three.0, one.0, "same enclosing function");
    assert!(one.2 >= one.1 && three.2 >= three.1, "non-empty op ranges");
    assert!(
        three.2 - three.1 > one.2 - one.1,
        "3 statements span more operators than 1: {one:?} vs {three:?}"
    );
    // The tail of the body (line 3 to the last statement) also maps.
    assert!(
        extract_range(F, "buf.loft", "default", 3, 5).is_some(),
        "lines 3-5 = b, c, return"
    );
}

#[test]
fn e1_refuses_non_statement_boundaries() {
    assert!(
        extract_range(F, "buf.loft", "default", 1, 1).is_none(),
        "the signature line is not a statement start"
    );
    assert!(
        extract_range(F, "buf.loft", "default", 6, 6).is_none(),
        "the closing brace is not a statement start"
    );
    assert!(
        extract_range(F, "buf.loft", "default", 4, 3).is_none(),
        "a reversed range"
    );
    assert!(
        extract_range(F, "buf.loft", "default", 99, 99).is_none(),
        "a line past EOF"
    );
}

// ── E2 — inputs (parameters): the upward-exposed uses of the slice ────────────

#[test]
fn e2_inputs_are_upward_exposed_uses() {
    // 1 fn f(x: integer) -> integer {
    // 2   y = x + 1;
    // 3   z = y * 2;
    // 4   return z;
    // 5 }
    // Selecting lines 2-3: `x` is read before any write in the slice (and is a
    // parameter → live-in) → an input; `y` is written then read → not an input;
    // `z` is written, never read → not an input.
    let f = "fn f(x: integer) -> integer {\n  y = x + 1;\n  z = y * 2;\n  return z;\n}\n";
    assert_eq!(
        extract_inputs(f, "buf.loft", "default", 2, 3),
        Some(vec!["x".to_string()]),
        "the only upward-exposed use is `x`"
    );
    // Selecting just line 3 (`z = y * 2`): now `y` is read before written → an input.
    assert_eq!(
        extract_inputs(f, "buf.loft", "default", 3, 3),
        Some(vec!["y".to_string()]),
        "narrowing to `z = y * 2` makes `y` the input"
    );
}

#[test]
fn e2_compound_assignment_reads_before_it_writes() {
    // `x = x + 1` is Set(x, Add(Var(x), 1)) — the RHS reads `x` BEFORE the write, so
    // `x` is upward-exposed (an input), even though it is also written in the slice.
    let f = "fn f(x: integer) -> integer {\n  x = x + 1;\n  return x;\n}\n";
    assert_eq!(
        extract_inputs(f, "buf.loft", "default", 2, 2),
        Some(vec!["x".to_string()]),
        "a compound write reads its target first → an input"
    );
}

#[test]
fn e2_a_slice_local_is_not_an_input() {
    // Selecting lines 2-3 where both `a` and `b` are written-first in the slice →
    // no inputs (nothing is read before being written).
    let f = "fn f() -> integer {\n  a = 1;\n  b = a + 2;\n  return b;\n}\n";
    assert_eq!(
        extract_inputs(f, "buf.loft", "default", 2, 3),
        Some(vec![]),
        "a, b are both produced in the slice → no inputs"
    );
}

// ── E3 — outputs (returns): writes in the slice that are live-out ─────────────

#[test]
fn e3_outputs_are_writes_that_are_live_out() {
    // 1 fn g(n: integer) -> integer {
    // 2   total = 0;
    // 3   for i in 0..n {
    // 4     total = total + i;
    // 5   }
    // 6   return total;
    // 7 }
    // Extracting the loop (lines 3-5): `total` is written in the slice and read
    // after it (`return total`) → an output; and it is also an input (read in the
    // body) → an IN-OUT parameter.
    let g = "fn g(n: integer) -> integer {\n  total = 0;\n  for i in 0..n {\n    total = total + i;\n  }\n  return total;\n}\n";
    let outs = extract_outputs(g, "buf.loft", "default", 3, 5).expect("the loop maps");
    assert!(
        outs.contains(&"total".to_string()),
        "total is live-out: {outs:?}"
    );
    // `total` is both an input and an output → in-out.
    let ins = extract_inputs(g, "buf.loft", "default", 3, 5).expect("the loop maps");
    assert!(
        ins.contains(&"total".to_string()) && outs.contains(&"total".to_string()),
        "total is an in-out param: ins={ins:?} outs={outs:?}"
    );
}

#[test]
fn e3_a_dead_write_is_not_an_output() {
    // 1 fn f(x: integer) -> integer {
    // 2   y = x + 1;    <- written in the slice, NEVER read after → not an output
    // 3   z = x + 2;    <- written and read after (return z) → an output
    // 4   return z;
    // 5 }
    let f = "fn f(x: integer) -> integer {\n  y = x + 1;\n  z = x + 2;\n  return z;\n}\n";
    let outs = extract_outputs(f, "buf.loft", "default", 2, 3).expect("the slice maps");
    assert_eq!(
        outs,
        vec!["z".to_string()],
        "only z is live-out (y is dead): {outs:?}"
    );
}

#[test]
fn e3_multiple_live_out_writes_become_multiple_outputs() {
    // Both `a` and `b` are written in the slice and read after it → two outputs.
    // 1 fn f() -> integer {
    // 2   a = 1;
    // 3   b = 2;
    // 4   return a + b;
    // 5 }
    let f = "fn f() -> integer {\n  a = 1;\n  b = 2;\n  return a + b;\n}\n";
    let outs = extract_outputs(f, "buf.loft", "default", 2, 3).expect("the slice maps");
    assert_eq!(outs.len(), 2, "a and b are both live-out: {outs:?}");
    assert!(
        outs.contains(&"a".to_string()) && outs.contains(&"b".to_string()),
        "{outs:?}"
    );
}

// ── E4 — synthesise the new function + the call, behaviour-preserving ─────────

#[test]
fn e4_extraction_is_behaviour_preserving_single_output() {
    // Extract `y = x + 1; z = y * 2` out of f: input `x`, output `z`.
    let f = "fn f(x: integer) -> integer {\n  y = x + 1;\n  z = y * 2;\n  return z;\n}\n\
             fn main() { print(\"{f(5)}\"); }\n";
    let e = extract_function(f, "buf.loft", "default", 2, 3).expect("extracts");
    eprintln!("NEW FN:\n{}\nCALL: {}", e.new_function, e.call);
    assert!(
        e.new_function
            .contains("fn extracted(x: integer) -> integer"),
        "signature: {}",
        e.new_function
    );
    assert!(e.call.contains("z = extracted(x)"), "call: {}", e.call);
    // Applying the edit re-parses clean AND runs identically.
    let applied = apply(f, &e);
    let diags = loft::lsp::diagnose(&applied, "buf.loft", "default");
    let errs: Vec<_> = diags
        .entries()
        .iter()
        .filter(|d| d.level >= loft::diagnostics::Level::Error)
        .collect();
    assert!(
        errs.is_empty(),
        "extracted program is clean:\n{applied}\n{errs:?}"
    );
    assert_eq!(run(f), run(&applied), "same output before/after extraction");
}

#[test]
fn e4_extraction_is_behaviour_preserving_loop_in_out() {
    // Extract the loop out of g: `total` is an in-out param, `n` an input.
    let g = "fn g(n: integer) -> integer {\n  total = 0;\n  for i in 0..n {\n    total = total + i;\n  }\n  return total;\n}\n\
             fn main() { print(\"{g(4)}\"); }\n";
    let e = extract_function(g, "buf.loft", "default", 3, 5).expect("extracts the loop");
    eprintln!("NEW FN:\n{}\nCALL: {}", e.new_function, e.call);
    // `total` is both a parameter and the return value (in-out).
    assert!(
        e.new_function.contains("total: integer"),
        "total param: {}",
        e.new_function
    );
    assert!(
        e.call.contains("total = extracted("),
        "call reassigns total: {}",
        e.call
    );
    let applied = apply(g, &e);
    let diags = loft::lsp::diagnose(&applied, "buf.loft", "default");
    let errs: Vec<_> = diags
        .entries()
        .iter()
        .filter(|d| d.level >= loft::diagnostics::Level::Error)
        .collect();
    assert!(
        errs.is_empty(),
        "extracted program is clean:\n{applied}\n{errs:?}"
    );
    assert_eq!(run(g), run(&applied), "same output before/after extraction");
}

// ── E5 — ownership / deps: refuse a &-reference, allow owned heap by value ─────

#[test]
fn e5_refuses_a_reference_parameter_input() {
    // `v` is a `&`-reference param used (and rebound) in the slice — its aliasing
    // can't be safely re-expressed as a by-value parameter, so extraction is refused.
    let f = "fn f(v: &vector<integer>) -> integer {\n  v = [9];\n  a = v.len();\n  return a;\n}\n";
    assert!(
        extract_function(f, "buf.loft", "default", 2, 3).is_none(),
        "a &-reference input is refused (E5 safety)"
    );
}

#[test]
fn e5_owned_heap_by_value_is_allowed_and_preserves_behaviour() {
    // An owned struct is copied into the parameter and delivered back via the return
    // (in-out) — value semantics make it safe.  Extract `p.x = p.x + 1` (modifies p).
    let f = "struct P { x: integer, y: integer }\nfn f(p: P) -> integer {\n  p.x = p.x + 1;\n  return p.x + p.y;\n}\n\
             fn main() { q = P{x: 3, y: 4}; print(\"{f(q)}\"); }\n";
    let e = extract_function(f, "buf.loft", "default", 3, 3).expect("owned struct is allowed");
    // p is an in-out param (its type is `P`, not `&P`).
    assert!(
        e.new_function.contains("p: P"),
        "struct param by value: {}",
        e.new_function
    );
    let applied = apply(f, &e);
    let diags = loft::lsp::diagnose(&applied, "buf.loft", "default");
    let errs: Vec<_> = diags
        .entries()
        .iter()
        .filter(|d| d.level >= loft::diagnostics::Level::Error)
        .collect();
    assert!(
        errs.is_empty(),
        "extracted program is clean:\n{applied}\n{errs:?}"
    );
    assert_eq!(
        run(f),
        run(&applied),
        "same output before/after extracting a struct in-out"
    );
}

// ── E6 — self threading + control-flow refusals ──────────────────────────────

#[test]
fn e6_refuses_control_flow_that_escapes_the_slice() {
    // A `return` inside the slice would leave the EXTRACTED function, not `f`, so the
    // extraction is refused (it would change control flow — a broken edit otherwise).
    let f = "fn f(x: integer) -> integer {\n  if x > 0 {\n    return x;\n  }\n  return 0;\n}\n";
    assert!(
        extract_function(f, "buf.loft", "default", 2, 4).is_none(),
        "a return inside the slice is refused (E6)"
    );
    // A WHOLE for-loop is still extractable — its internal iterator break is inside
    // the loop, so it does not escape the slice.
    let g = "fn g(n: integer) -> integer {\n  total = 0;\n  for i in 0..n {\n    total = total + i;\n  }\n  return total;\n}\n";
    assert!(
        extract_function(g, "buf.loft", "default", 3, 5).is_some(),
        "a whole for-loop extracts (its break is inside the loop)"
    );
}

#[test]
fn e6_method_extraction_threads_self_and_preserves_behaviour() {
    // `self` is a normal input parameter; extracting a method's statements works and
    // is behaviour-preserving.
    let m = "struct P { x: integer, y: integer }\nfn area(self: P) -> integer {\n  a = self.x * self.y;\n  b = a + 1;\n  return b;\n}\n\
             fn main() { q = P{x: 3, y: 4}; print(\"{q.area()}\"); }\n";
    let e = extract_function(m, "buf.loft", "default", 3, 4).expect("method statements extract");
    assert!(
        e.new_function.contains("self: P"),
        "self is threaded as a parameter: {}",
        e.new_function
    );
    let applied = apply(m, &e);
    let diags = loft::lsp::diagnose(&applied, "buf.loft", "default");
    let errs: Vec<_> = diags
        .entries()
        .iter()
        .filter(|d| d.level >= loft::diagnostics::Level::Error)
        .collect();
    assert!(
        errs.is_empty(),
        "extracted program is clean:\n{applied}\n{errs:?}"
    );
    assert_eq!(
        run(m),
        run(&applied),
        "same output before/after extracting from a method"
    );
}

#[test]
fn e1_refuses_a_selection_inside_a_nested_block() {
    // 1  fn g() {
    // 2    total = 0;
    // 3    for i in 0..3 {
    // 4      total = total + i;    <- inside the loop body (a nested block)
    // 5    }
    // 6  }
    let g = "fn g() {\n  total = 0;\n  for i in 0..3 {\n    total = total + i;\n  }\n}\n";
    // Line 4 is inside the loop body → NOT a top-level statement → refused.
    assert!(
        extract_range(g, "buf.loft", "default", 4, 4).is_none(),
        "a line inside a nested loop body is not a top-level statement"
    );
    // But the top-level statements DO map: `total = 0` (line 2), and the whole
    // `for` statement (lines 3-5) as one top-level operator group.
    assert!(
        extract_range(g, "buf.loft", "default", 2, 2).is_some(),
        "the top-level `total = 0`"
    );
    assert!(
        extract_range(g, "buf.loft", "default", 3, 5).is_some(),
        "the whole for statement"
    );
}
