// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! W1.9 — WASM `compile_and_run()` entry point tests.
//!
//! The Rust-side unit tests verify the virtual FS API directly.
//! The Node.js smoke test is provided as an ignored test with a shell command.

use loft::file_access as fa;

extern crate loft;

/// W1.9 (native): virt_fs populate, get, and clear round-trip.
///
/// This exercises the thread-local VIRT_FS helpers without needing WASM.
#[cfg(feature = "wasm")]
#[test]
fn virt_fs_roundtrip() {
    use loft::wasm::{virt_fs_clear, virt_fs_get, virt_fs_populate};

    virt_fs_populate(&[
        ("main.loft".to_string(), "fn main() {}".to_string()),
        (
            "helper.loft".to_string(),
            "fn greet() -> text { \"hi\" }".to_string(),
        ),
    ]);

    assert_eq!(virt_fs_get("main.loft").as_deref(), Some("fn main() {}"));
    assert_eq!(
        virt_fs_get("helper.loft").as_deref(),
        Some("fn greet() -> text { \"hi\" }")
    );
    assert!(virt_fs_get("missing.loft").is_none());

    virt_fs_clear();
    assert!(virt_fs_get("main.loft").is_none());
}

/// W1.9 (Node.js integration test): runs the full WASM bridge test suite.
///
/// Requires:
///   1. `wasm-pack build --target nodejs --out-dir tests/wasm/pkg \
///          -- --no-default-features --features wasm`
///   2. Node.js in PATH.
///
/// Skips gracefully when either prerequisite is absent.
#[test]
fn wasm_compile_and_run_smoke() {
    // Skip if the WASM package is not built.
    if !fa::exists(std::path::Path::new("tests/wasm/pkg/loft.js")) {
        println!("SKIP wasm_compile_and_run_smoke — WASM package not built");
        println!(
            "     Run: wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --no-default-features --features wasm"
        );
        return;
    }

    // Skip if Node.js is not in PATH.
    let node_check = loft::platform::process::harness_command("node")
        .arg("--version")
        .output();
    if node_check.is_err() {
        println!("SKIP wasm_compile_and_run_smoke — node not in PATH");
        return;
    }

    // Run the bridge test suite.
    let result = loft::platform::process::harness_command("node")
        .arg("tests/wasm/bridge.test.mjs")
        .status()
        .expect("failed to launch node");

    assert!(
        result.success(),
        "WASM bridge tests failed (exit {:?}) — run `node tests/wasm/bridge.test.mjs` for details",
        result.code()
    );
}

/// @PLN149 step 8 — the JSON surface the doc-site panel drives.
///
/// One session end to end, through the two exported entries and nothing else: start over a
/// source, list what there is to call, break on a LINE (what a gutter click sends), pause,
/// read a live local, call the program's own function against the frame, and resume.
///
/// Native, because the entries are ordinary functions — only the `wasm_bindgen` attribute
/// is behind the feature — so the contract is checkable without a browser.  The `output`
/// field is the one part that is not: print is captured only in a browser build, so it is
/// empty here and the browser harness is what checks it carries the program's output.
#[test]
fn debug_session_json_surface() {
    use loft::wasm_debug::{debug_command, debug_start};
    let src = "fn fib(n: integer) -> integer { if n < 2 { return n; } fib(n-1) + fib(n-2) }\n\
               fn main() {\n  a = fib(9);\n  b = a * 2;\n  print(\"a={a} b={b}\\n\");\n}\n";
    assert_eq!(debug_start(src), "{\"ok\":true}");

    // The callable list is the program's own functions — not `main`, and not the stdlib.
    let fns = debug_command("fns");
    assert!(
        fns.contains("fib(n: integer) -> integer"),
        "the program's function, with its signature: {fns}"
    );
    assert!(
        !fns.contains("main("),
        "main is not something to call: {fns}"
    );

    // A LINE breakpoint, which is what a gutter click sends: line 4 is `b = a * 2;`, so
    // `a` is assigned by now and `b` is not.
    assert!(debug_command("bp 4").contains("D:ok bp 4"));
    let hit = debug_command("run");
    assert!(hit.contains("D:hit main"), "paused in main: {hit}");
    assert!(hit.contains("a=34"), "fib(9) is 34, and it is live: {hit}");
    assert!(
        hit.contains("b=<unset>"),
        "and b is not assigned yet: {hit}"
    );
    assert!(
        !hit.contains("__work"),
        "the compiler's scratch is not the reader's variables: {hit}"
    );

    // Evaluate against the paused frame: a live local, an expression over it, and the
    // program's own function — the last is the one the panel exists for.
    assert!(debug_command("eval a").contains("D:eval a=34"));
    assert!(debug_command("eval a * 2").contains("D:eval a * 2=68"));
    assert!(debug_command("eval fib(6)").contains("D:eval fib(6)=8"));

    assert!(
        debug_command("resume").contains("D:terminated"),
        "runs to completion"
    );
}

/// A source that does not compile answers with the reason, not a bare failure — a page that
/// will not run the reader's code has to say why.
#[test]
fn debug_start_reports_why_it_did_not_compile() {
    let bad = loft::wasm_debug::debug_start("fn main() { nosuchfunction(1); }\n");
    assert!(bad.starts_with("{\"ok\":false"), "refused: {bad}");
    assert!(
        bad.contains("nosuchfunction"),
        "the diagnostic names the offending call: {bad}"
    );
}

/// A command with no session answers, rather than going quiet — the page can lose its
/// session (a reload, a failed start) and has to render something.
#[test]
fn debug_command_without_a_session_says_so() {
    // A failed start leaves no session behind.
    let _ = loft::wasm_debug::debug_start("fn main() { nosuchfunction(1); }\n");
    let r = loft::wasm_debug::debug_command("run");
    assert!(r.contains("no session"), "answers without a session: {r}");
}

/// @PLN149 step 8 — the panel session against the REAL wasm module.
///
/// [`debug_session_json_surface`] above checks the grammar and the values natively, and
/// cannot check the one field that exists only in a wasm build: `output`. Print is captured
/// by `crate::wasm::output_push`, compiled in only under the `wasm` feature, so natively
/// that field is present and always empty — a regression that stopped a page's output
/// reaching its panel would pass every native test in this file. `tools/doc_panel_check.mjs`
/// runs the same session against the built module and asserts the output arrives.
///
/// Skips (exit 2) when the package is not built or predates the two exports, the same way
/// [`wasm_compile_and_run_smoke`] does: not having built it is not a wrong answer.
#[test]
fn doc_panel_session_runs_on_wasm() {
    if !fa::exists(std::path::Path::new("tests/wasm/pkg/loft.js")) {
        println!("SKIP doc_panel_session_runs_on_wasm — WASM package not built");
        println!(
            "     Run: wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --no-default-features --features wasm"
        );
        return;
    }
    if loft::platform::process::harness_command("node")
        .arg("--version")
        .output()
        .is_err()
    {
        println!("SKIP doc_panel_session_runs_on_wasm — node not in PATH");
        return;
    }
    let out = loft::platform::process::harness_command("node")
        .arg("tools/doc_panel_check.mjs")
        .output()
        .expect("failed to launch node");
    if out.status.code() == Some(2) {
        println!(
            "SKIP doc_panel_session_runs_on_wasm — {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        return;
    }
    assert!(
        out.status.success(),
        "the doc panel session failed on wasm:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
