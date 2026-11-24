// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN203 rust-writer: the Rust emitter's own output — its native runs name `--native-rust`.

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

use loft::file_access as fa;

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
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn baseline_present() -> bool {
    fa::exists(std::path::Path::new(BASELINE_DIR))
        && CORPUS.iter().all(|t| {
            let name = fa::file_stem(t).unwrap();
            fa::exists(std::path::Path::new(BASELINE_DIR).join(format!("{name}.rs")))
        })
}

fn emit_native(loft_src: &str, out_path: &std::path::Path) {
    let status = loft::platform::process::harness_command(loft_binary())
        .args([
            "--native",
            "--native-emit",
            out_path.to_str().unwrap(),
            loft_src,
        ])
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
    let _ = fa::create_dir_all(&tmp_dir);
    let mut diffs: Vec<String> = Vec::new();
    for src in CORPUS {
        let name = fa::file_stem(src).unwrap();
        let out = tmp_dir.join(format!("{name}.rs"));
        emit_native(src, &out);
        let baseline = std::path::Path::new(BASELINE_DIR).join(format!("{name}.rs"));
        let actual = fa::read_to_string(&out).expect("read emitted .rs");
        let expected = fa::read_to_string(&baseline).expect("read baseline .rs");
        if actual != expected {
            diffs.push(name);
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
    let status = loft::platform::process::harness_command(loft_binary())
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
    let src = fa::read_to_string(baseline).expect("read 13-file baseline");
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
    fa::write(
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

    let out = loft::platform::process::harness_command(loft_binary())
        .args(["--introspect", src.to_str().unwrap()])
        .current_dir(project_root())
        .output()
        .expect("failed to spawn loft binary — run `cargo build --release` first");
    let _ = fa::remove_file(&src);
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
    fa::write(
        &src,
        "fn main() {\n\
         \x20   t = (([1, 2], 1), 5);\n\
         \x20   u = t;\n\
         \x20   println(\"{len(u.0.0)}\");\n\
         }\n",
    )
    .expect("write nested-tuple source");

    let out = loft::platform::process::harness_command(loft_binary())
        .args(["--introspect", src.to_str().unwrap()])
        .current_dir(project_root())
        .output()
        .expect("failed to spawn loft binary — run `cargo build --release` first");
    let _ = fa::remove_file(&src);
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
    let status = loft::platform::process::harness_command(loft_binary())
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
    let status = loft::platform::process::harness_command(loft_binary())
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
    let status = loft::platform::process::harness_command(loft_binary())
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
        let name = fa::file_stem(test).unwrap_or_else(|| test.to_string());
        let path = baseline.join(format!("{name}.rs"));
        let src = match fa::read_to_string(&path) {
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

// ============================================================
// Wart-budget gates (plan 09 phase 00 evaluation findings)
// ============================================================

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
    let src = fa::read_to_string(project_root().join("src/data.rs")).expect("read data.rs");
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
