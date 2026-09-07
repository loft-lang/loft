// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan 09 phase 00 step 0.8 — emitter dispatch validation suite.
//!
//! Runs against the doc-test corpus baseline captured at
//! `/tmp/p09-baseline/*.rs` (created by `scripts/p09_fast_gate.sh
//! --capture`).  Confirms phase 00's two contracts:
//!
//! 1. Every Op-emission call site routes through `emit_op` and
//!    falls through to `DefaultEmitter` (registry empty).  The
//!    generated source is byte-identical to the pre-phase-09
//!    emission.
//! 2. P203's let-bind-on-repeat (step 0.7b shipped earlier) stays
//!    closed: the reproducer exits 0 under native.
//!
//! When a custom emitter is later registered, this suite is the
//! safety net that catches divergence between the new emission and
//! the byte-identical baseline.  Each entry in `BASELINE_CORPUS`
//! that's affected by a new custom emitter should be regenerated
//! intentionally and re-captured.

extern crate loft;

use std::process::Command;

const CORPUS: &[&str] = &[
    "tests/docs/03-integer.loft",
    "tests/docs/04-boolean.loft",
    "tests/docs/07-vector.loft",
    "tests/docs/08-struct.loft",
    "tests/docs/13-file.loft",
    "tests/docs/19-threading.loft",
    "tests/docs/25-generics.loft",
    // Phase 03: parallel-for emission coverage.
    "tests/scripts/22-threading.loft",
    // Phase 04: OpGetRecord / OpIterate emission coverage.
    "tests/docs/10-sorted.loft",
];

const BASELINE_DIR: &str = "/tmp/p09-baseline";

fn project_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn loft_binary() -> std::path::PathBuf {
    project_root().join("target/release/loft")
}

fn baseline_present() -> bool {
    std::path::Path::new(BASELINE_DIR).exists()
        && CORPUS.iter().all(|t| {
            let name = std::path::Path::new(t)
                .file_stem()
                .unwrap()
                .to_string_lossy();
            std::path::Path::new(BASELINE_DIR)
                .join(format!("{name}.rs"))
                .exists()
        })
}

fn emit_native(loft_src: &str, out_path: &std::path::Path) {
    let status = Command::new(loft_binary())
        .args(["--native-emit", out_path.to_str().unwrap(), loft_src])
        .current_dir(project_root())
        .status()
        .expect("failed to spawn loft binary — run `cargo build --release` first");
    assert!(
        status.success(),
        "--native-emit failed for {loft_src} (exit {})",
        status.code().unwrap_or(-1)
    );
}

/// Phase 00 contract 1: every doc-test in CORPUS produces byte-identical
/// emission compared to the baseline captured before phase 00 started.
///
/// The baseline lives at `/tmp/p09-baseline/`.  Capture (or refresh) via
/// `scripts/p09_fast_gate.sh --capture`.  When no baseline is present,
/// this test skips with an explanatory message — running locally without
/// having captured the baseline shouldn't fail the suite.
#[test]
fn baseline_emission_unchanged() {
    if !baseline_present() {
        eprintln!(
            "[codegen_emitter] no baseline at {BASELINE_DIR}; \
             run `scripts/p09_fast_gate.sh --capture` to seed.  Skipping."
        );
        return;
    }
    let tmp_dir = std::env::temp_dir().join("p09-codegen-emitter-test");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let mut diffs: Vec<String> = Vec::new();
    for src in CORPUS {
        let name = std::path::Path::new(src)
            .file_stem()
            .unwrap()
            .to_string_lossy();
        let out = tmp_dir.join(format!("{name}.rs"));
        emit_native(src, &out);
        let baseline = std::path::Path::new(BASELINE_DIR).join(format!("{name}.rs"));
        let actual = std::fs::read_to_string(&out).expect("read emitted .rs");
        let expected = std::fs::read_to_string(&baseline).expect("read baseline .rs");
        if actual != expected {
            diffs.push(name.into_owned());
        }
    }
    assert!(
        diffs.is_empty(),
        "phase 00 byte-identical contract broken — diverging files: {diffs:?}.  \
         Either fix the emission, or (if intentional) refresh the baseline \
         via `scripts/p09_fast_gate.sh --capture`."
    );
}

/// Phase 00 step 0.7b regression guard — P203 stays closed.
///
/// The `OpConvIntFromEnum` template at `default/01_code.loft:705`
/// substitutes `@v1` twice; before the let-bind-on-repeat fix, the
/// assertion `delete(path) == FileResult.Ok` called `n_delete()` twice
/// and panicked.  The fix in `output_call_template` hoists repeated
/// placeholders into a single `let _v_<name>` binding.  This test
/// guards against regression.
#[test]
fn p203_reproducer_passes_under_native() {
    // Use the pre-built `target/release/loft` directly, not
    // `cargo run --bin loft --release`.  Nested cargo invocations
    // inside `cargo test --release` take the target-tree lock and
    // can re-check the dep graph, racing against the parallel rustc
    // workers in tests/native.rs::native_dir / native_scripts and
    // producing `mold: Operation not permitted` linker failures.
    // The other reproducer tests below (p204 / p200 / p244) use
    // the same direct-invocation pattern.
    let status = Command::new(loft_binary())
        .arg("tests/scripts/repro_p203.loft")
        .current_dir(project_root())
        .status()
        .expect("failed to spawn loft binary — run `cargo build --release` first");
    assert!(
        status.success(),
        "P203 reproducer failed under native (exit {}) — \
         the let-bind-on-repeat in calls.rs::output_call_template \
         may have regressed",
        status.code().unwrap_or(-1)
    );
}

/// Phase 00 step 0.7b structural guard — the affected templates do
/// produce a `let _v_<name>` binding shape in their generated code,
/// proving the let-bind-on-repeat path is active.  If this test ever
/// reports zero matches, the pre-pass in `output_call_template` was
/// silently disabled.
#[test]
fn let_bind_on_repeat_appears_in_emission() {
    if !baseline_present() {
        eprintln!("[codegen_emitter] no baseline; skipping let-bind-on-repeat structural check");
        return;
    }
    // tests/docs/13-file.loft uses the `delete(...) == FileResult.X`
    // pattern that triggers `OpConvIntFromEnum`'s let-bind-on-repeat.
    let baseline = std::path::Path::new(BASELINE_DIR).join("13-file.rs");
    let src = std::fs::read_to_string(baseline).expect("read 13-file baseline");
    assert!(
        src.contains("let _v_v1"),
        "13-file.rs baseline lacks `let _v_v1` — let-bind-on-repeat may not be \
         engaging for repeated @v1 placeholders.  Re-capture the baseline if \
         the emission shape changed intentionally."
    );
}

/// @PLN118 arc E — the interpreter must re-sentinel an owned-reassigned
/// variable after its block-exit free, mirroring the native backend
/// (`generation/ops/ref_ops.rs` resets `store_nr = u16::MAX` after every
/// var free).  A struct variable reassigned each loop iteration from a
/// fresh call (`p = mk(i)`) takes an unconditional pre-build `OpFreeRef`
/// AND a block-exit `OpFreeRef`; without an `OpInitRefSentinel` after the
/// block-exit free, a loop re-entry's pre-build free re-frees a slot the
/// allocator may have reclaimed for a live value — the moros glb H4
/// store-slot-reuse UAF (`pos` reads null on 4-of-7 hex verts, 3/32
/// exported accessors).
///
/// Why this guards the FIX rather than the SYMPTOM: the corruption is
/// layout-fragile and only reproduces on the external moros `5e677b7`
/// scene — the in-tree `demo_village` fixture exports 0/32 even with the
/// fix reverted, so no in-suite symptom repro exists.  The fix's emitted
/// signature is deterministic, though: this minimal program emits exactly
/// one `InitRefSentinel` with the fix and ZERO without it (verified by
/// neutralizing `owned_reassigned.insert` in `generate_call` and
/// re-introspecting).  Asserting the sentinel is present is the
/// non-vacuous guard.
#[test]
fn pln118_arce_owned_reassign_emits_sentinel() {
    let dir = std::env::temp_dir();
    let src = dir.join("pln118_arce_owned_reassign.loft");
    std::fs::write(
        &src,
        "struct P { x: integer }\n\
         fn mk(v: integer) -> P { P { x: v } }\n\
         fn main() {\n\
         \x20   p = mk(0);\n\
         \x20   for i in 0..3 { p = mk(i); }\n\
         \x20   println(\"{p.x}\");\n\
         }\n",
    )
    .expect("write arc-E source");

    let out = Command::new(loft_binary())
        .args(["--introspect", src.to_str().unwrap()])
        .current_dir(project_root())
        .output()
        .expect("failed to spawn loft binary — run `cargo build --release` first");
    let _ = std::fs::remove_file(&src);
    let ir = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "introspect failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        ir.contains("InitRefSentinel"),
        "arc-E fix missing: the owned-reassigned `p`, freed at block-exit, is not \
         followed by an `InitRefSentinel`, so a loop re-entry re-frees a reused \
         slot (moros glb H4 store-slot-reuse UAF).  Neutralizing \
         `owned_reassigned.insert` in `generate_call` drops this to zero.  IR:\n{ir}"
    );
}

/// Gate: a NESTED tuple's copy must not free the source it reads.
///
/// `tuple_member_owned_copy` holds the inner tuple in a `_tuphold` local and
/// copies each of its heap members out individually.  That hold MINTS NOTHING —
/// unlike the keyed, struct and vector branches beside it, which each allocate a
/// backing and are rightly created with their deps stripped, because an empty dep
/// list is `@FR-O-Proxy`'s proxy for "this binding owns its store".  Created the
/// same way, the hold read as an owner and the scope pass derived a free for it,
/// so `u = t` emitted `OpFreeRef(_tuphold_1.0)` and destroyed `t`'s own vector
/// (`@FR-O-Borrow`: a value that aliases another is tracked and never frees).
///
/// Why this guards the FIX rather than the SYMPTOM: the freed record keeps its
/// bytes, so the interpreter, `--native` and the ordinary suite all read the RIGHT
/// answer off freed memory — a value assertion is vacuous for this class.  Only the
/// nightly `LOFT_POISON` gate saw it, as 0xDEADBEEF arriving in `vector_append`.
/// The emitted signature is deterministic: this program emits ZERO
/// `OpFreeRef(_tuphold` with the fix and one without it (verified by restoring
/// `owned_create` at that branch and re-introspecting; the same edit puts the free
/// back in ten other probe shapes).
#[test]
fn nested_tuple_copy_does_not_free_its_source() {
    let dir = std::env::temp_dir();
    let src = dir.join("nested_tuple_hold_borrows.loft");
    std::fs::write(
        &src,
        "fn main() {\n\
         \x20   t = (([1, 2], 1), 5);\n\
         \x20   u = t;\n\
         \x20   println(\"{len(u.0.0)}\");\n\
         }\n",
    )
    .expect("write nested-tuple source");

    let out = Command::new(loft_binary())
        .args(["--introspect", src.to_str().unwrap()])
        .current_dir(project_root())
        .output()
        .expect("failed to spawn loft binary — run `cargo build --release` first");
    let _ = std::fs::remove_file(&src);
    let ir = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "introspect failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Non-vacuous in both directions: the hold must still be BUILT (or the program
    // stopped taking this path and the free-count below would pass for free)...
    assert!(
        ir.contains("_tuphold"),
        "the nested-tuple copy no longer builds a `_tuphold`, so this guard has \
         stopped measuring what it names — re-derive it against \
         `tuple_member_owned_copy`.  IR:\n{ir}"
    );
    // ...and nothing may free what that hold only borrows.
    assert!(
        !ir.contains("OpFreeRef(_tuphold"),
        "a nested tuple's copy frees the source it reads: `_tuphold` only NAMES the \
         inner tuple so each member can be copied out, so freeing its member destroys \
         the SOURCE's heap member (`u = t` frees `t.0.0`).  The value read back stays \
         correct on both backends, so only LOFT_POISON sees it at runtime.  IR:\n{ir}"
    );
}

// ============================================================
// Phase 01 ABI consolidation gates
// ============================================================

/// Gate: the duplicate hardcoded `LEGACY_STORES_FNS` lists that lived
/// in `src/generation/calls.rs` and `src/generation/dispatch.rs` must
/// stay deleted.  Plan 09 phase 01 replaced them with a single
/// `crate::codegen_runtime::abi_of(name)` lookup.
///
/// If a future change re-introduces `LEGACY_STORES_FNS` (the typical
/// quick fix when adding a new legacy-ABI runtime fn), this test
/// fails — the right answer is to add the entry to
/// `CODEGEN_RUNTIME_FNS` in `src/codegen_runtime.rs` instead.
#[test]
fn no_hardcoded_abi_lists_remain() {
    for path in &["src/generation/calls.rs", "src/generation/dispatch.rs"] {
        let src = std::fs::read_to_string(project_root().join(path)).expect("read source file");
        // Allow doc-comment references to the historical name; only flag
        // actual `const LEGACY_STORES_FNS` declarations.
        assert!(
            !src.contains("const LEGACY_STORES_FNS"),
            "{path} reintroduced `const LEGACY_STORES_FNS` — \
             plan 09 phase 01 retired this in favour of \
             `crate::codegen_runtime::abi_of(name)`.  Add new legacy-ABI \
             runtime fns to `CODEGEN_RUNTIME_FNS` in `src/codegen_runtime.rs`."
        );
    }
}

/// Phase 04 structural test: `OpGetRecord` and `OpIterate` emission
/// moved out of dispatch.rs::output_call_inner's special-case match
/// into registered emitters in `src/generation/ops/key_ops.rs`.
/// Re-introducing match arms for these names is a regression.
#[test]
fn no_key_op_special_case_in_dispatch() {
    let src = std::fs::read_to_string(project_root().join("src/generation/dispatch.rs"))
        .expect("read dispatch.rs");
    for op in &["OpGetRecord", "OpIterate"] {
        let pat = format!("\"{op}\"");
        let has_arm = src.lines().any(|line| {
            let t = line.trim();
            t.starts_with(&pat) && t.contains("=>")
        });
        assert!(
            !has_arm,
            "src/generation/dispatch.rs reintroduced an `\"{op}\" => …` \
             match arm.  Phase 04 retired this in favour of the registered \
             emitter in `src/generation/ops/key_ops.rs`.  New key-keyed \
             Op variants should register custom emitters there, not add \
             match arms to dispatch.rs."
        );
    }
}

/// Phase 03 structural test: `n_parallel_for` / `n_parallel_for_light`
/// emission moved out of dispatch.rs::output_call_inner's special-case
/// match into the registered `ParallelForEmitter`.  Re-introducing
/// the special case is a regression of phase 03's structural intent —
/// new parallel variants should register custom emitters instead.
#[test]
fn no_parallel_special_case_in_dispatch() {
    let src = std::fs::read_to_string(project_root().join("src/generation/dispatch.rs"))
        .expect("read dispatch.rs");
    // Allow doc-comment references to the historical name; only flag
    // actual match-arm patterns `"n_parallel_for"` followed by `=>`.
    // The deleted arm was `"n_parallel_for" | "n_parallel_for_light" =>`.
    let has_arm = src.lines().any(|line| {
        let t = line.trim();
        t.starts_with("\"n_parallel_for\"") && t.contains("=>")
    });
    assert!(
        !has_arm,
        "src/generation/dispatch.rs reintroduced an `\"n_parallel_for\" => …` \
         match arm.  Phase 03 retired this in favour of the registered \
         `ParallelForEmitter` in `src/generation/ops/parallel.rs` — new \
         parallel variants should register custom emitters there, not \
         add match arms to dispatch.rs."
    );
}

/// Phase 09 structural test: the three `n_parallel_for_*_native`
/// public fns must remain thin wrappers around
/// `n_parallel_for_native_core`.  Re-inflating any of them (e.g. by
/// inlining shape-specific alloc/dispatch/merge logic into the
/// public fn body) regresses phase 09's consolidation — that
/// duplication is what phase 06 (queue variants) needs the shape
/// trait to absorb.  New parallel variants should add a `ParShape`
/// impl and call the generic core, not re-inline the skeleton.
#[test]
fn parallel_runtime_consolidated() {
    let src = std::fs::read_to_string(project_root().join("src/codegen_runtime.rs"))
        .expect("read codegen_runtime.rs");
    for fn_name in [
        "n_parallel_for_native",
        "n_parallel_for_text_native",
        "n_parallel_for_ref_native",
    ] {
        let needle = format!("pub fn {fn_name}<F>(");
        let start = src
            .find(&needle)
            .unwrap_or_else(|| panic!("did not find `pub fn {fn_name}<F>(` in codegen_runtime.rs"));
        // Body starts after the first `{` past the signature.
        let body_open = start
            + src[start..]
                .find('{')
                .expect("function signature missing opening brace");
        // Walk to the matching close brace.
        let mut depth = 0i32;
        let mut body_close = body_open;
        for (i, ch) in src[body_open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        body_close = body_open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        assert!(
            body_close > body_open,
            "{fn_name}: failed to locate closing brace"
        );
        let body_lines = src[body_open + 1..body_close].lines().count();
        assert!(
            body_lines <= 15,
            "{fn_name} body is {body_lines} lines — phase 09 consolidation \
             expects each public parallel-for fn to be a thin wrapper (≤ 15 \
             body lines) calling `n_parallel_for_native_core` with a `ParShape`. \
             If you've added shape-specific logic, extend the `ParShape` trait \
             instead of re-inlining the skeleton."
        );
        assert!(
            src[body_open..body_close].contains("n_parallel_for_native_core"),
            "{fn_name} body must call `n_parallel_for_native_core` (phase 09 \
             consolidation).  Direct re-implementation regresses the shape \
             trait and inflates phase 06's queue-variant duplication."
        );
    }
}

/// Phase 06 regression test: the queue + buf-access runtime fns
/// must remain registered in `CODEGEN_RUNTIME_FNS` with `Abi::Cell`
/// tags.  P202's reproducer (19_threading + 22_threading +
/// 40_par_ref_return) compiles natively only when these fns exist
/// at link time — accidentally removing them re-opens the bug.
#[test]
fn p202_parallel_queue_runtime_fns_registered() {
    use loft::codegen_runtime::{Abi, abi_of};
    for name in [
        "n_parallel_queue_native",
        "n_parallel_queue_text_native",
        "n_parallel_queue_ref_native",
        "n_parallel_buf_get_native",
        "n_parallel_buf_get_text_native",
        "n_parallel_buf_get_ref_native",
        "n_parallel_buf_drop_native",
        "n_parallel_buf_drop_text_native",
        "n_parallel_buf_drop_ref_native",
    ] {
        assert_eq!(
            abi_of(name),
            Abi::Cell,
            "{name} must be in CODEGEN_RUNTIME_FNS with Abi::Cell — \
             phase 06 (P202 close) requires it for native compilation \
             of `for ... par(...)` loops."
        );
    }
}

/// Phase 06 structural test: the `n_parallel_queue` family must
/// route through the `ParallelQueueEmitter` registered in
/// `src/generation/ops/mod.rs`.  Without this, the call sites emit
/// fixed-arity stub calls instead of closure-shaped helper calls,
/// reproducing P202's E0061 compile failure.
#[test]
fn p202_parallel_queue_emitter_registered() {
    let src = std::fs::read_to_string(project_root().join("src/generation/ops/mod.rs"))
        .expect("read ops/mod.rs");
    for name in [
        "\"n_parallel_queue\"",
        "\"n_parallel_queue_text\"",
        "\"n_parallel_queue_ref\"",
        "\"n_parallel_buf_get\"",
        "\"n_parallel_buf_get_text\"",
        "\"n_parallel_buf_get_ref\"",
        "\"n_parallel_buf_drop\"",
        "\"n_parallel_buf_drop_text\"",
        "\"n_parallel_buf_drop_ref\"",
    ] {
        assert!(
            src.contains(name),
            "build_registry missing entry for {name} — phase 06 \
             (P202 close) registered ParallelQueueEmitter / \
             ParallelBufRenameEmitter for these names; removing \
             the entry re-opens E0061 on native par-queue tests."
        );
    }
}

/// Phase 10 step 10.5 — native baseline floor.
///
/// Pins the post-plan-09 native suite count.  Without this gate,
/// future commits can silently regress (e.g. a codegen change
/// that breaks a previously-passing test goes unnoticed because
/// the high-level test still reports "FAILED" the same way).
///
/// Floor today (after plan-09 phases 06 + 07 + 10 step 10.3 closed
/// P200/P202/P205):
/// - native_dir:           30/30
/// - native_scripts:       91/93 (2 P204 sub-failures remaining;
///   close via plan-11 to reach 93/93)
/// - native_binary_script: PASS
/// - native_tuple_*:       PASS (× 2)
///
/// Update floor expectations as plan-11 (P204) ships.  This
/// test's job: catch silent regression — any commit that drops
/// the count below today's floor must update the floor
/// EXPLICITLY in this test, not silently.
#[test]
#[ignore = "expensive — runs the full native suite; opt-in via `--ignored`"]
fn native_suite_floor_holds() {
    let output = std::process::Command::new("cargo")
        .args([
            "test",
            "--release",
            "--test",
            "native",
            "--",
            "--test-threads=1",
        ])
        .current_dir(project_root())
        .output()
        .expect("run native suite");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}{}", stdout, String::from_utf8_lossy(&output.stderr));

    // The native test binary has 5 high-level tests
    // (`native_binary_script`, `native_dir`, `native_scripts`,
    // `native_tuple_return_script`, `native_tuple_script`).  All
    // 5 must pass.  Per the zero-regression rule, this floor only
    // moves UP — when a future plan adds a new high-level test
    // that becomes part of the parity baseline, update the
    // count here EXPLICITLY.
    //
    // The cargo test summary line is emitted on every run:
    //   test result: ok. 5 passed; 0 failed; ...
    // We check for the specific "5 passed; 0 failed" pattern.
    //
    // History:
    // - Pre-plan-09: 2/5 high-level tests passed.
    // - After plan-09 phase 06 (P202): 3/5.
    // - After plan-09 phase 07 (P205): 3/5 (still failing on P200 + P204).
    // - After plan-09 phase 10 step 10.3 (P200): 4/5.
    // - After plan-11 (P204): 5/5 (parity with main).
    let high_level_floor = combined.contains("5 passed; 0 failed")
        || combined.contains("6 passed; 0 failed")  // future expansion
        || combined.contains("7 passed; 0 failed");
    assert!(
        high_level_floor,
        "native suite high-level floor regressed (expected ≥ 5/5 passing, 0 failing).  \
         Plan-09 + plan-11 closed P200/P202/P203/P204/P205; the parity-with-main floor \
         is 5/5.  If you've added a new high-level test, update this floor explicitly.\n\nStdout:\n{combined}"
    );

    // Per-test FAILED check — even if the high-level count looks
    // OK, no individual test should report FAILED status.
    for name in [
        "native_dir",
        "native_binary_script",
        "native_scripts",
        "native_tuple_return_script",
        "native_tuple_script",
    ] {
        assert!(
            !combined.contains(&format!("test {name} ... FAILED")),
            "{name} regressed under native — high-level test reports FAILED.  Stdout:\n{combined}"
        );
    }
}

/// Plan-11 regression test: P204 — tail-expression `return inner_call()`
/// from a struct-returning function must compile + run cleanly under
/// native.  Without plan-11's fix, the Call's result is discarded as
/// a void statement and the function returns the null sentinel —
/// callers' `OpCopyRecord(_src, ...)` then panics with index out of
/// bounds when `_src.store_nr == u16::MAX`.
#[test]
fn p204_tail_expression_return_passes_under_native() {
    // Direct binary invocation — see p203_reproducer_passes_under_native
    // for the nested-cargo race rationale.
    let status = std::process::Command::new(loft_binary())
        .arg("tests/scripts/repro_p204.loft")
        .current_dir(project_root())
        .status()
        .expect("run repro_p204 under native");
    assert!(
        status.success(),
        "P204: tail-expression-return reproducer failed under native — \
         the detect_ref_tail_capture fix in src/generation/pre_eval.rs \
         likely needs review."
    );
}

/// Phase 10 regression test: P200's read-side comparison emission
/// fix must keep `tests/scripts/20-binary.loft` compiling under
/// native.  Without phase 10's `IntCompareEmitter` (in
/// `src/generation/ops/int_compare.rs`), the default `@v1 == @v2`
/// template produces E0308 between a narrowed block-tail LHS
/// (`(...) as u8`) and an `_i64` literal RHS.  The emitter wraps
/// both operands in `(... as i64)` to normalise to a common width.
#[test]
fn p200_binary_compiles_under_native() {
    // Direct binary invocation — see p203_reproducer_passes_under_native
    // for the nested-cargo race rationale.
    let status = std::process::Command::new(loft_binary())
        .arg("tests/scripts/20-binary.loft")
        .current_dir(project_root())
        .status()
        .expect("run 20-binary under native");
    assert!(
        status.success(),
        "P200: 20-binary.loft native compile/run regressed — \
         the IntCompareEmitter widening fix in \
         src/generation/ops/int_compare.rs likely needs review."
    );
}

/// Phase 10 structural test: the IntCompareEmitter must remain
/// registered for all four integer comparison Ops.  Removing any
/// entry re-opens P200 (and any other site where a narrow LHS
/// meets an i64 RHS literal).
#[test]
fn p200_int_compare_emitter_registered() {
    let src = std::fs::read_to_string(project_root().join("src/generation/ops/mod.rs"))
        .expect("read ops/mod.rs");
    for name in ["\"OpEqInt\"", "\"OpNeInt\"", "\"OpLtInt\"", "\"OpLeInt\""] {
        assert!(
            src.contains(name),
            "build_registry missing entry for {name} — phase 10 \
             (P200 close) registered IntCompareEmitter for these \
             names; removing the entry re-opens E0308 on \
             narrowed-LHS comparisons."
        );
    }
}

/// P244 regression test: any program that links a native sub-crate
/// declaring a `text`-returning native (e.g. `lib/server`'s
/// `n_ws_message`, `n_ws_event_payload`, `n_tcp_method`, etc.) must
/// compile + run cleanly under default `--native`.  Without the
/// `needs_text_wrap` branch in `output_native_direct_call`, rustc
/// rejects every wrapper for these natives with E0308 ("expected
/// `Str`, found `LoftStr`") because the wrapper signature is `Str`
/// but the underlying extern returns `loft_ffi::LoftStr`.  Fix:
/// extract the LoftStr bytes, push into `stores.scratch`, and
/// return a `Str` borrowed from there (mirrors P205 lifetime
/// pattern).  Type annotation on the temporary is omitted so the
/// "multiple versions of crate loft_ffi" case (one per native
/// sub-crate) doesn't surface as a sibling E0308.
// @P389 (cross-package --native link on Linux CI, OPEN) blocks
// this test under registry-installed cdylibs.  Server depends on
// web, so the test pulls in two chunk-resident cdylibs in one
// link — the exact shape @P389 covers (rustls rlib not visible
// across package boundaries).  Before net Stage B (Phase 6b)
// the CI pre-built lib/web/native + lib/server/native from the
// monorepo, populating each `target/release/deps/` with the
// transitive rlibs and masking the cross-package link issue.
// Post-Stage B both packages live in `~/.loft/build-cache/`
// and the link fails.
//
// The actual @P244 regression target (the LoftStr→String wrapper
// in `src/generation/mod.rs::output_native_direct_call`) is
// not at risk from @P389 — wrapper codegen happens before link.
// (@PLN10 N2 changed that wrapper from a scratch-backed `Str` to
// an owned `String`; the emit-only shape is pinned by
// `pln10_n2_cdylib_text_wrapper_returns_owned_string` below, which
// also covers the emit half standalone.)  The ORIGINAL blocker —
// @P389 / issue #249, cross-package native link — closed 2026-06-04,
// It runs UNDER `cargo test` too.  The nested native link used to hit "found crates
// (`loft_ffi` and `loft_ffi`) with colliding StableCrateId values" and the test was
// ignored for it; the C-ABI dylib link seals the package's crate graph (WINDOWS.md,
// verified 2026-06-15), so that class is gone and this is an ordinary test again.
#[test]
fn p244_text_native_wrapper_compiles_under_native() {
    // Direct binary invocation — see p203_reproducer_passes_under_native
    // for the nested-cargo race rationale.
    let status = std::process::Command::new(loft_binary())
        .arg("tests/integration/p244_smoke.loft")
        .current_dir(project_root())
        .status()
        .expect("run lib/server smoke under native");
    assert!(
        status.success(),
        "P244: lib/server smoke test failed under default --native — \
         the LoftStr→String wrapper fix in src/generation/mod.rs \
         (output_native_direct_call::needs_text_wrap branch) regressed."
    );
}

/// @PLN10 N2 regression — a cdylib `#native` text-returning function's
/// generated wrapper returns an OWNED `String` (no `stores.scratch`).
///
/// Pre-N2 `output_native_direct_call`'s `needs_text_wrap` branch pushed the
/// foreign `LoftStr` bytes into the never-cleared `stores.scratch` buffer and
/// handed back a borrowed `Str` (a program-lifetime leak).  N2 returns the
/// bytes as an owned `String`, bridged to the caller via `Deref` (@P304), and
/// flips the wrapper signature to `-> String` (gated by `!def.native.is_empty()`).
///
/// Emit-only (`--native-emit`): wrapper codegen runs before any link, so this
/// pins the shape even though @P389 blocks the full cross-package build of the
/// `server` smoke (the only in-repo consumer with text-returning cdylib natives).
/// Skips gracefully when the `server` package isn't resolvable on this box.
#[test]
fn pln10_n2_cdylib_text_wrapper_returns_owned_string() {
    let out_rs = std::env::temp_dir().join(format!("loft_pln10_n2_{}.rs", std::process::id()));
    let status = std::process::Command::new(loft_binary())
        .arg("--native-emit")
        .arg(&out_rs)
        .arg("tests/integration/p244_smoke.loft")
        .current_dir(project_root())
        .status()
        .expect("run --native-emit on the server smoke");
    if !status.success() {
        eprintln!(
            "skipping pln10_n2_cdylib_text_wrapper_returns_owned_string — \
             `server` package not resolvable (--native-emit exited non-zero)"
        );
        return;
    }
    let emitted = std::fs::read_to_string(&out_rs).expect("read emitted native source");
    let _ = std::fs::remove_file(&out_rs);

    // n_tcp_path is a representative text-returning cdylib native
    // (server-0.1.1/src/server.loft:46 → loft_server::n_tcp_path).
    let after = &emitted[emitted
        .find("fn n_tcp_path(")
        .expect("generated source must contain the n_tcp_path wrapper")..];
    let body = &after[..after.find("\n}").map_or(after.len(), |e| e + 2)];
    assert!(
        body.contains("-> String"),
        "N2: cdylib text-native wrapper must return owned `String`, got:\n{body}"
    );
    assert!(
        !body.contains("scratch"),
        "N2: cdylib text-native wrapper must NOT touch `stores.scratch`, got:\n{body}"
    );
    assert!(
        body.contains("String::from_utf8_unchecked"),
        "N2: cdylib text-native wrapper must materialise the bytes as an owned \
         `String`, got:\n{body}"
    );
}

/// @P310 regression: native codegen must emit `*const i64` (not `*const i32`)
/// for a `vector<integer>` argument bound to a `vec<i64>` FFI wrapper.  The
/// graphics `Canvas.data` (`vector<integer>`, 8-byte stride post-2c) is
/// passed to `loft_save_png(data: vec<i64>)`; before the fix
/// `vector_elem_rust_type` keyed off `is_wide()` (a value-range predicate)
/// and emitted `*const i32`, so every graphics program failed `--check` /
/// `--native` with E0308.  The fix keys off the storage stride
/// (`vector_narrow_width()`); the unit test
/// `generation::p310_vector_elem_tests` pins it at the function level, this
/// pins it end-to-end (a real graphics program must `--check` clean).
#[test]
fn p310_graphics_vector_ffi_checks_clean() {
    // Direct binary invocation — see p203_reproducer_passes_under_native
    // for the nested-cargo race rationale.  The fixture is its own mini-
    // package (`tests/fixtures/p310/loft.toml`) with a registry dep on
    // `graphics`; loft auto-installs on first `use graphics;`.  Previously
    // this test passed `--lib lib` to link the WHOLE monorepo native stack
    // (graphics + web + server + imaging) under one rustc, doubling as the
    // Windows multi-lib link guard — but @PLAN12 Phase 5b moved those four
    // packages to chunk repos, so the cross-lib guard now lives in the
    // graphics chunk's own CI matrix (per-platform builds against pinned
    // loft).  This test still pins the vector<integer> → *const i64 path
    // end-to-end against a real `graphics::save_png` consumer.
    let out = std::process::Command::new(loft_binary())
        .args(["--check", "tests/fixtures/p310/p310_save_png.loft"])
        .current_dir(project_root())
        .output()
        .expect("run --check on the @P310 graphics save_png fixture");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The C-ABI native path links the graphics native package's cdylib, which
    // must RESOLVE at link time — but graphics needs a system GL library that a
    // GL-less macOS/Windows CI runner doesn't have ("needs the system library
    // 'libGL.so.1'" → undefined `loft_save_png`).  That is ENVIRONMENTAL, not
    // the storage-stride regression this test guards: the stride bug is a rustc
    // TYPE error (E0308 / `*const i32`) on the GENERATED source that fires at
    // COMPILE, before the link.  So where graphics native can't link, still
    // verify the codegen is clean and skip only the link-success assertion;
    // assert full success everywhere graphics native CAN link (Linux CI).
    if !out.status.success() && stderr.contains("needs the system library") {
        assert!(
            !stderr.contains("E0308") && !stderr.contains("*const i32"),
            "P310: the vector_elem_rust_type storage-stride fix regressed (E0308 \
             on the generated source) — vector<integer> must lower to *const i64.  \
             stderr={stderr:?}"
        );
        eprintln!(
            "p310_graphics_vector_ffi_checks_clean: skipped link-success \
             (graphics native needs a system GL library absent on this runner)"
        );
        return;
    }
    assert!(
        out.status.success(),
        "P310: graphics save_png fixture failed `--check`.  The \
         vector_elem_rust_type storage-stride fix (src/generation/mod.rs) \
         likely regressed — vector<integer> must lower to *const i64 to \
         match the vec<i64> `loft_save_png` wrapper, not *const i32.  \
         stderr={stderr:?}"
    );
}

/// Phase 07 regression test: the P205 reproducer must compile +
/// run cleanly under native.  Pins the dangling-Str fix —
/// without phase 07's emit-time scratch routing, the generic-text-
/// return path emits `Str::new(&local_String)` whose pointer
/// dies at function return, and the assert in
/// `tests/scripts/repro_p205.loft:18` fails at runtime.
#[test]
fn p205_repro_passes_under_native() {
    // Direct binary invocation — see p203_reproducer_passes_under_native
    // for the nested-cargo race rationale.
    let status = std::process::Command::new(loft_binary())
        .arg("tests/scripts/repro_p205.loft")
        .current_dir(project_root())
        .status()
        .expect("run repro_p205 under native");
    assert!(
        status.success(),
        "P205: bounded-generic text return reproducer failed under native — \
         the dangling Str::new(&local_String) fix in src/generation/emit.rs \
         (Value::Return wrap_text path + wrap_result block path) regressed."
    );
}

/// Phase 07 structural test: the dangling shape `Str::new(&_local_*)`
/// must NOT appear in any generated Rust for the doc-test corpus.
/// If it reappears, the P205 fix has regressed and bounded-generic
/// text returns are dangling pointers again.
#[test]
fn p205_no_str_new_of_local_in_corpus() {
    if !baseline_present() {
        eprintln!("skipping p205_no_str_new_of_local_in_corpus — baseline absent");
        return;
    }
    let baseline = std::path::Path::new(BASELINE_DIR);
    let mut offenders: Vec<String> = Vec::new();
    for test in CORPUS {
        let name = std::path::Path::new(test)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(test);
        let path = baseline.join(format!("{name}.rs"));
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // The dangle pattern is `Str::new(&var___ret_<digit>` (or any
        // local String-typed binding the generic-text-return code
        // synthesises).  Phase 07's fix routes those through
        // `stores.scratch.last().unwrap()` instead, so no `Str::new(&var_`
        // borrow-of-local should remain.
        for (i, line) in src.lines().enumerate() {
            if line.contains("Str::new(&var___ret_") {
                offenders.push(format!("{name}.rs:{}: {}", i + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "P205 regression: generated code reintroduced `Str::new(&var___ret_*)` \
         dangling-pointer pattern.  Phase 07's fix routes generic-text-returns \
         through `stores.scratch` to escape this; reintroducing the borrow-of-local \
         shape is a regression.\n\nOffenders:\n  {}",
        offenders.join("\n  ")
    );
}

/// Gate: the registry's `Abi` tags must be self-consistent — every
/// entry's `abi_of(name)` lookup must return the entry's own tag.
/// Trivially true when the registry is well-formed; catches a typo
/// where two entries with the same name disagree on ABI.
#[test]
fn abi_of_handles_all_runtime_fns() {
    use loft::codegen_runtime::{Abi, CODEGEN_RUNTIME_FNS, abi_of};
    for fn_def in CODEGEN_RUNTIME_FNS {
        assert_eq!(
            abi_of(fn_def.name),
            fn_def.abi,
            "abi_of disagrees with registry for `{}`",
            fn_def.name
        );
    }
    // Unknown name → Cell (user-fn / Op-stub default).
    assert_eq!(abi_of("nonexistent_fn_for_test"), Abi::Cell);
}

// ============================================================
// Wart-budget gates (plan 09 phase 00 evaluation findings)
// ============================================================

/// Gate A: the special-case Op `match` in `dispatch.rs::output_call_inner`
/// has been ELIMINATED — every Op now dispatches through the `emit_op`
/// registry (`src/generation/ops/`) or a `#rust` template.  This gate is now
/// a ratchet at **zero**: it fails if anyone re-introduces a `"Op…" =>` match
/// arm.  New Op-specific native emission must be a registered `OpEmitter` or a
/// `#rust` template, never a match arm.
const DISPATCH_OP_ARM_BUDGET: usize = 0;

#[test]
fn dispatch_op_arm_budget_not_exceeded() {
    let src = std::fs::read_to_string(project_root().join("src/generation/dispatch.rs"))
        .expect("read dispatch.rs");
    let start = src
        .find("fn output_call_inner")
        .expect("output_call_inner not found in dispatch.rs");
    // Find the closing `}` of the function body — heuristic: scan until
    // the next `^    }$` (4-space indent + brace) line after start.
    let after_start = &src[start..];
    let body_end_rel = after_start
        .match_indices("\n    }\n")
        .next()
        .map(|(i, _)| i)
        .unwrap_or(after_start.len());
    let body = &after_start[..body_end_rel];

    // Count match-arm patterns: a line whose trimmed prefix begins with
    // `"Op` and contains `=>` is an Op match arm.  This is a heuristic
    // but resilient to the multi-pattern `"Op…" | "Op…" =>` form
    // because each line still starts with a string literal.
    let arms = body
        .lines()
        .filter(|line| {
            let t = line.trim();
            t.starts_with("\"Op") && t.contains("=>")
        })
        .count();

    assert!(
        arms == DISPATCH_OP_ARM_BUDGET,
        "dispatch.rs::output_call_inner has {arms} `\"Op…\" =>` match arm(s) — the \
         match was eliminated and must stay at 0.  Add native Op emission as a \
         registered `OpEmitter` impl in `src/generation/ops/` (or a `#rust` \
         template in default/*.loft), not as a match arm."
    );
}

/// Gate B: caps the set of "codegen-only" `Value` variants — IR
/// variants that are produced exclusively by native code generation
/// and have no parser source or runtime semantics.
///
/// `Value::RawExpr` (added by phase 00 step 0.7) is the sole sanctioned
/// codegen-only variant.  Adding more is a wart: the IR loses meaning
/// because variants exist purely as plumbing for codegen synthesis.
/// Each codegen-only variant requires no-op default arms in every
/// walker (parser, scopes, pre_eval, state codegen) — the cost grows
/// linearly with each addition.
///
/// **Rule**: if you need to thread synthesized values through codegen,
/// build a string-aware companion entry point rather than another
/// `Value` variant.  Plan 09 phase 00 evaluation documented this
/// constraint; see `doc/claude/plans/finished/09-native-runtime-rewrite/00-scaffold.md`
/// "Findings (post-completion)".
const SANCTIONED_CODEGEN_VALUE_VARIANTS: &[&str] = &["RawExpr"];

#[test]
fn no_unsanctioned_codegen_value_variants() {
    let src = std::fs::read_to_string(project_root().join("src/data.rs")).expect("read data.rs");
    // Find the Value enum body.
    let start = src.find("pub enum Value {").expect("Value enum not found");
    let end_rel = src[start..]
        .match_indices("\n}")
        .next()
        .map(|(i, _)| i)
        .unwrap_or(src.len() - start);
    let body = &src[start..start + end_rel];

    // Count occurrences of the "codegen-internal" / "codegen-only"
    // marker string.  Each sanctioned variant's docstring contains
    // exactly one occurrence.  If new variants add the marker, the
    // count exceeds the sanctioned-list length.
    let marker_lines = body
        .lines()
        .filter(|l| l.contains("codegen-internal") || l.contains("codegen-only"))
        .count();

    // Each sanctioned variant must be present in the enum.
    for v in SANCTIONED_CODEGEN_VALUE_VARIANTS {
        assert!(
            body.contains(&format!("{v}(")),
            "sanctioned codegen-only variant `{v}` not found in Value enum"
        );
    }

    // No additional codegen-only variants allowed.  The marker may
    // appear multiple times in a single variant's docstring (we use
    // `<=` rather than `==` to tolerate prose mentions like
    // "this variant is codegen-internal" appearing twice).  But
    // adding a NEW variant with the marker bumps the count above
    // a tolerated ceiling and breaks this gate.
    let max_tolerated = SANCTIONED_CODEGEN_VALUE_VARIANTS.len() * 5;
    assert!(
        marker_lines <= max_tolerated,
        "codegen-internal marker appears {marker_lines} times in Value enum; \
         expected at most {max_tolerated} (sanctioned list = {SANCTIONED_CODEGEN_VALUE_VARIANTS:?}).  \
         A new codegen-only variant likely landed.  Either remove it (preferred — use a \
         string-aware companion entry point) or add it to SANCTIONED_CODEGEN_VALUE_VARIANTS \
         AND document why in plan 09 phase 00's scaffold doc."
    );
}

/// Plan-12 phase 01: structural guard against the Span-miss walker
/// pattern that caused P204.
///
/// `pre_eval.rs::patch_hoisted_returns` and `value_mentions_var` walk
/// the IR matching against `Value::Set(_, _)`, `Value::Return(_)`, etc.
/// directly.  When the parser wraps an operator in `Value::Span(box
/// (pos, inner))` the bare `matches!` falls through to `_ => false /
/// no match`, so the walker silently bails on Span-wrapped IR.  Plan-11
/// fixed one such site in `detect_ref_tail_capture`; plan-12 phase 01
/// fixes the rest.
///
/// This test scans `pre_eval.rs::patch_hoisted_returns` for `matches!`
/// against `Value::*` variants and asserts each is paired with
/// `.unspan()`.  Heuristic gate — doesn't catch every form — but
/// prevents the obvious regressions that would re-open P204-style
/// bugs.
#[test]
fn pre_eval_walkers_unspan() {
    let src = std::fs::read_to_string(project_root().join("src/generation/pre_eval.rs"))
        .expect("read pre_eval.rs");
    // Slice patch_hoisted_returns + value_mentions_var (the area the
    // plan-12 audit covers).  The deeper sites in
    // collect_pre_evals_inner / output_call handle Span via the existing
    // recursive helpers (`needs_pre_eval`, `create_stack_var`) and
    // intentionally don't unspan at every leaf.
    let start = src
        .find("fn patch_hoisted_returns")
        .expect("patch_hoisted_returns marker missing");
    let end_marker = "fn op_uses_stores";
    let end = src[start..]
        .find(end_marker)
        .map(|o| start + o)
        .expect("op_uses_stores marker missing");
    let region = &src[start..end];
    let mut offenders: Vec<(usize, &str)> = Vec::new();
    for (n, line) in region.lines().enumerate() {
        // Detect raw `matches!(op, Value::*` or `match v.as_ref()`
        // forms that don't already unspan.
        let trimmed = line.trim_start();
        let raw_matches_op = (trimmed.contains("matches!(op,") || trimmed.contains("matches!(op."))
            && !trimmed.contains("op.unspan(")
            && trimmed.contains("Value::");
        let raw_value_var = trimmed.contains("Value::Var")
            && trimmed.contains("inner.as_ref()")
            && !trimmed.contains(".unspan(");
        if raw_matches_op || raw_value_var {
            offenders.push((n, line));
        }
    }
    assert!(
        offenders.is_empty(),
        "Plan-12 phase 01: {} walker site(s) in pre_eval.rs::patch_hoisted_returns \
         match against Value::* without `.unspan()`.  This re-opens the P204-style \
         Span-miss bug class.  Add `.unspan()` before each match (see plan-11 P204 \
         fix and plan-12 phase 01 doc for the rationale).  Offenders:\n  {}",
        offenders.len(),
        offenders
            .iter()
            .map(|(n, l)| format!("L{}: `{}`", n + 1, l.trim()))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
