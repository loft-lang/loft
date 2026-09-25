// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Binary-level exit-code tests for L7.
//!
//! These tests invoke the compiled `loft` binary via `std::process::Command` so
//! they can verify the OS exit code — something the library-level test harness
//! cannot do.  The binary must be rebuilt (`cargo test` does this automatically
//! for integration tests).

use std::process::Command;
#[cfg(unix)]
use std::process::Stdio;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A program with no diagnostics must run and exit 0.
/// 46-caveats.loft is a clean caveat regression suite that should print "caveats: all ok".
#[test]
fn warning_only_program_exits_zero() {
    let script = workspace_root().join("tests/scripts/46-caveats.loft");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "expected exit 0 for warnings-only program, got {:?}; stdout={stdout:?}; stderr={stderr:?}",
        out.status.code()
    );
    assert!(
        stdout.contains("caveats: all ok"),
        "expected 'caveats: all ok' in output; got {stdout:?}"
    );
}

/// A program with a genuine parse error must exit non-zero.
#[test]
fn parse_error_exits_nonzero() {
    // Write a minimal syntax-error script to a temp file.
    let dir = std::env::temp_dir();
    let path = dir.join("loft_l7_test_parse_error.loft");
    std::fs::write(&path, "fn main() { x = 1\n").expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    assert!(
        !out.status.success(),
        "expected non-zero exit for parse-error program, got exit 0"
    );
}

/// `loft test` on a file with a compile ERROR reports that error, as a plain run does.  The runner
/// used to run the scope pass regardless, and a test holding `break` outside a loop panicked
/// inside it (`index out of bounds`), so the file read *"scope check panic"* and the author never
/// saw *"Cannot break outside a loop"*.  The program path has always stopped at an error.
#[test]
fn loft_test_reports_a_compile_error_instead_of_a_scope_panic() {
    let dir = std::env::temp_dir().join(format!("loft_test_break_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("break_outside.loft");
    std::fs::write(&path, "fn test_wrong_break() {\n  break;\n}\n").expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("test")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_dir_all(&dir);
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success(),
        "a refused file must fail the run: {all}"
    );
    assert!(
        all.contains("Cannot break outside a loop"),
        "the error must reach the reader: {all}"
    );
    assert!(
        !all.contains("panic"),
        "no internal panic may stand in for it: {all}"
    );
}

/// An unresolvable `#native` symbol (no cdylib provides it) must surface a LOUD
/// diagnostic at LOAD time — naming the symbol and how to rebuild — not stay
/// silent until a generic panic at first call.  The warning is non-fatal: a
/// declared-but-never-called native still lets the program run (exit 0), so the
/// operator learns which library to rebuild without the program being aborted.
#[test]
fn unresolved_native_warns_at_load_not_at_call() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "loft_unresolved_native_{}.loft",
        std::process::id()
    ));
    // The #native is declared but never called: the load-time warning must fire
    // anyway, and the program must still reach exit 0.
    std::fs::write(
        &path,
        "pub fn ghost_fn(x: integer) -> integer;\n\
         #native \"loft_ghost_nonexistent_symbol\"\n\n\
         fn main() { print(\"ran fine\"); }\n",
    )
    .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "uncalled unresolved native must not abort the program; got exit {:?}\nstderr: {stderr}",
        out.status.code()
    );
    assert!(
        stdout.contains("ran fine"),
        "program body must still run; stdout: {stdout}"
    );
    assert!(
        stderr.contains("did not load") && stderr.contains("loft_ghost_nonexistent_symbol"),
        "expected a load-time diagnostic naming the unresolved symbol; stderr: {stderr}"
    );
}

// ── P131: Loft CLI forwards script-level arguments (FIXED) ─────────────────
//
// `src/main.rs` now treats every token after the script path — including
// `--*` ones — as a script argument that is appended to `user_args` and
// forwarded to the script's `arguments()`. An explicit `--` separator is
// also accepted and skipped. The script must run cleanly when invoked
// with extra script-level arguments.
#[test]
fn p131_cli_forwards_script_dashdash_arg() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p131_args_test.loft");
    std::fs::write(&path, "fn main() { println(\"ran\"); }\n").expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .arg("--mode")
        .arg("glb")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "expected exit 0 with --mode forwarded; stdout={stdout:?}; stderr={stderr:?}"
    );
    assert!(
        stdout.contains("ran"),
        "expected script body to run; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// Explicit `--` separator must also be accepted (and consumed) before
/// script arguments.
#[test]
fn p131_cli_explicit_dashdash_separator() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p131_sep_test.loft");
    std::fs::write(&path, "fn main() { println(\"ran\"); }\n").expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .arg("--")
        .arg("--mode")
        .arg("glb")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    assert!(
        out.status.success(),
        "expected exit 0 with `--` separator; stderr={:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// P131: `arguments()` must return only the script-level arguments,
/// not the loft binary name or loft CLI flags like `--interpret`.
#[test]
fn p131_arguments_returns_only_script_args() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p131_arguments_content.loft");
    // Print each argument on its own line so we can inspect them.
    std::fs::write(&path, "fn main() { for a in arguments() { println(a) } }\n")
        .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .arg("--mode")
        .arg("glb")
        .arg("extra")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "expected exit 0; stderr={stderr:?}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec!["--mode", "glb", "extra"],
        "arguments() should return only script-level args, not loft flags; got: {lines:?}"
    );
}

// ── loft#684: a program argument that spells a subcommand reaches the program ──

/// loft#684: a positional argument after the script path belongs to the program,
/// even when it spells a loft subcommand (`layout`, `test`, `build`, …).  Before
/// the fix the CLI claimed the word and printed the usage line for an unrelated
/// subcommand, so the program never ran — and `--` did not stop it either.
///
/// The `--` cell also pins the end-of-options marker: after it, even a token the
/// CLI would otherwise recognise as its own flag is forwarded verbatim.
#[test]
fn issue684_subcommand_word_is_a_program_argument() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_issue684_subcommand_arg.loft");
    std::fs::write(&path, "fn main() { for a in arguments() { println(a) } }\n")
        .expect("write temp file");

    // Each cell: the args after the script path, and what `arguments()` must yield.
    let cells: [(&[&str], &[&str]); 4] = [
        // A plain word is unaffected — the control that proves the harness reads args.
        (&["hello", "world"], &["hello", "world"]),
        // The reported shape: a subcommand word as a later positional.
        (&["hello", "layout"], &["hello", "layout"]),
        // Several at once, one of them the very first program argument.
        (
            &["test", "build", "install", "fmt"],
            &["test", "build", "install", "fmt"],
        ),
        // `--` forwards everything after it, including a recognised loft flag.
        (&["--", "--lib", "layout"], &["--lib", "layout"]),
    ];

    for (args, expected) in cells {
        let out = Command::new(loft_bin())
            .arg("--interpret")
            .arg(&path)
            .args(args)
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "expected exit 0 for args {args:?}; stdout={stdout:?}; stderr={stderr:?}"
        );
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(
            lines, expected,
            "args {args:?} should reach the program verbatim; got {lines:?}"
        );
    }
    let _ = std::fs::remove_file(&path);
}

/// loft#684 guard: the fix must not stop a subcommand from working as the FIRST
/// positional — that is the only place a subcommand can appear.  Without this the
/// test above would pass on a CLI that had simply lost `loft layout` entirely.
#[test]
fn issue684_subcommand_still_works_as_first_positional() {
    let out = Command::new(loft_bin())
        .arg("layout")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("loft layout:"),
        "`loft layout` must still dispatch to the layout subcommand; got {combined:?}"
    );
}

// ── W1.1: --html produces a self-contained HTML file ──────────────────────

/// W1.1: `--html` must produce a valid HTML file with embedded WASM.
/// Requires the `wasm32-unknown-unknown` rustup target — skipped in CI
/// environments where the target is not installed.
#[test]
fn w1_1_html_export_produces_file() {
    let dir = std::env::temp_dir();
    let src = dir.join("loft_w1_1_test.loft");
    let out = dir.join("loft_w1_1_test.html");
    std::fs::write(&src, "fn main() { println(\"html-ok\"); }\n").unwrap();
    let result = Command::new(loft_bin())
        .arg("--html")
        .arg(&out)
        .arg(&src)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&src);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    if stderr.contains("wasm32-unknown-unknown") && stderr.contains("not be installed") {
        eprintln!("SKIP: wasm32-unknown-unknown target not installed");
        return;
    }
    assert!(
        result.status.success(),
        "expected --html to succeed; stdout={stdout:?}; stderr={stderr:?}"
    );
    let html = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    assert!(
        html.contains("<!DOCTYPE html>"),
        "HTML should start with doctype"
    );
    assert!(
        html.contains("loft_start"),
        "HTML should reference loft_start entry point"
    );
    // A compute program (no graphics) now gets the minimal engine-less page, so
    // the GL bridge (`buildLoftImports`) is absent by design.  The `loft_io`
    // output host import is present in BOTH the minimal and the full GL page —
    // assert that instead, to confirm the I/O bridge is wired.
    assert!(
        html.contains("loft_host_print"),
        "HTML should wire the loft_io host import"
    );
    // WASM binary is embedded as base64 — file should be substantial
    assert!(
        html.len() > 5000,
        "HTML too small ({} bytes) — WASM likely missing",
        html.len()
    );
}

// ── P171: --native mode OpCopyRecord panicked on 0x8000-tagged tp ─────────
//
// Root cause: `src/codegen_runtime.rs::OpCopyRecord` was missing the 0x8000
// "free source after copy" tag-bit masking that the bytecode equivalent
// (`src/state/io.rs::copy_record`, line 1021) applies.  Any caller setting
// the tag — e.g. `copy_ref` on a struct-returning call's result — caused
// an out-of-bounds panic at `Types::size()` (index 0x805B = 32859 into a
// 124-entry array).  Surfaced by running moros_render's `map_export_glb`
// path under `--native`.  Fix: port the mask + `remove_claims` call +
// free-source branch from the bytecode version.

/// P171: compiling and running `isolated_stair.loft` under `--native` must
/// complete without panic and produce the same output as the interpreter.
/// Guards a native-mode run through `map_export_glb` → `map_build_scene`
/// → OpCopyRecord with the 0x8000 tag set.
#[test]
fn p171_native_copy_record_high_bit_does_not_panic() {
    let script =
        workspace_root().join("tests/fixtures/libs/moros_render/examples/isolated_stair.loft");
    // The script writes `isolated_stair.glb` CWD-relative (portable — Windows
    // has no /tmp), so run it in a temp working dir and read the GLB from
    // there.  Mirrors the moros_glb_cli_end_to_end pattern below.
    let glb_path = std::env::temp_dir().join("isolated_stair.glb");
    let _ = std::fs::remove_file(&glb_path);
    let path_arg = format!("{}/", workspace_root().display());
    let out = Command::new(loft_bin())
        .arg("--native")
        .arg("--path")
        .arg(&path_arg)
        .arg(&script)
        .current_dir(std::env::temp_dir())
        .output()
        .expect("invoke loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Skip if rustc isn't available, the graphics native rlib isn't
    // compiled against the current rustc, or the rlib hasn't been
    // built at all — all three are environment issues, not
    // regressions.  E0514 = rustc version mismatch; E0463 = can't
    // find crate (rlib missing / `auto_build_native` couldn't run on
    // this runner, e.g. missing X11 headers for glutin).
    // E0308 with a `*const i32` vs `*const i64` pointer mismatch is
    // also environmental: the loft binary and `loft_graphics_native`
    // cdylib were built against different integer-width layouts
    // (typically a stale `target/release/loft` from before the i64
    // migration against a fresh cdylib, or vice versa).  A clean
    // rebuild of both crates resolves it; it is never a regression
    // in the code under test.
    // @P229 G2 (Windows LNK1181 windows-targets link-search) was fixed
    // 2026-05-30 in src/native_utils.rs, so the previous LNK1181 skip branch
    // is removed — this test now exercises the native multi-lib link on
    // Windows too.  The remaining skips are genuine toolchain-availability
    // cases (E0514 rustc-version mismatch, E0463 missing rlib, the i32/i64
    // layout-mismatch from a stale cdylib) — never code-under-test bugs.
    if stderr.contains("rustc not found")
        || stderr.contains("E0514")
        || stderr.contains("E0463")
        || (stderr.contains("E0308")
            && stderr.contains("*const i32")
            && stderr.contains("*const i64"))
    {
        eprintln!("SKIP: native toolchain not ready — {stderr}");
        return;
    }
    assert!(
        out.status.success(),
        "native run must exit 0; stdout={stdout:?}; stderr={stderr:?}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "native run must not panic; stderr={stderr:?}"
    );
    assert!(
        stdout.contains("mesh '1': 96 verts, 48 tris"),
        "output must match interpreter (96-vert default-rise stair); \
         stdout={stdout:?}"
    );
    // Verify the GLB the script wrote has the glTF magic.
    let glb = std::fs::read(&glb_path).expect("GLB written");
    assert_eq!(&glb[0..4], b"glTF", "GLB magic must be 'glTF'");
    let _ = std::fs::remove_file(&glb_path);
}

// ── tail-call ref-return capture with a store-lifetime "lifted" arg ────────
//
// Native-codegen regression (the zero-trust ztserve blocker; fix in
// `src/generation/emit.rs`, the `is_tail_capture_call` branch).  A ref-returning
// tail call captured into `__native_tail_ret` whose argument is an inline
// call-result that the store-lifetime pass LIFTS emitted the lift as a leading
// `{ … };` statement; its own `;` terminated the capture `let` early — binding the
// var to the lift's `()` and detaching the call (rustc E0308 "expected DbRef, found
// ()").  The fix wraps the capture value in a block (lift = statement, call = tail).

/// Native-codegen regression: a captured ref-return tail call with a store-lifetime
/// "lifted" inline-call argument must compile + run under `--native`.  Mirrors the
/// zero-trust `opsurface::handle_write_s` shape `resp_frame(rid, tag, empty_body())`.
#[test]
fn tail_capture_lifted_arg_compiles_native() {
    let script = workspace_root().join("tests/scripts/tail-capture-lifted-arg.loft");
    let out = Command::new(loft_bin())
        .arg("--native")
        .arg(&script)
        .output()
        .expect("invoke loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Skip on toolchain-availability issues (not code-under-test regressions),
    // mirroring p171 above.
    if stderr.contains("rustc not found") || stderr.contains("E0514") || stderr.contains("E0463") {
        eprintln!("SKIP: native toolchain not ready — {stderr}");
        return;
    }
    assert!(
        out.status.success(),
        "native run must exit 0 (the lifted-arg tail capture must not E0308); \
         stdout={stdout:?}; stderr={stderr:?}"
    );
    assert!(
        stdout.contains("ok"),
        "expected the script's 'ok' (its assert passed); stdout={stdout:?}"
    );
}

// ── (I-Join): an inferred integer local widens to the join of its writes ───
//
// The #433-residual (formal/types.md was D4).  `arg = b[0]` (vector<u8>) infers
// `arg : u8`; `arg = arg*256 + b[1]` assigns an `integer` that doesn't fit u8.
// Pre-fix `arg` stayed u8 and the wider write errored on both backends (E0308 on
// native for the cbor shape).  An inferred local now takes the join (`integer`);
// an annotated `arg: u8` would still be constrained.

/// (I-Join) regression: an inferred multiply-assigned integer local compiles + runs
/// natively, widened to the join of its writes (not the narrowest first one).
#[test]
fn ijoin_multiply_assigned_widens_native() {
    let script = workspace_root().join("tests/scripts/433-ijoin-multiply-assigned.loft");
    let out = Command::new(loft_bin())
        .arg("--native")
        .arg(&script)
        .output()
        .expect("invoke loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stderr.contains("rustc not found") || stderr.contains("E0514") || stderr.contains("E0463") {
        eprintln!("SKIP: native toolchain not ready — {stderr}");
        return;
    }
    assert!(
        out.status.success(),
        "native run must exit 0 (the inferred local must widen to the join, not narrow); \
         stdout={stdout:?}; stderr={stderr:?}"
    );
    assert!(
        stdout.contains("ok"),
        "expected the script's 'ok' (second(v) == 258); stdout={stdout:?}"
    );
}

// ── P166 / loft#829: .content() on a binary file — warn, and answer null ──
//
// Root-cause data-loss bug: prior to the 2026-04-17 fix,
// `file("x.glb").content()` silently returned "" on any file whose bytes
// failed UTF-8 decode — `src/state/io.rs::get_file_text`'s `read_to_string`
// failure path called `buf.clear()` with no log.  P166 added the stderr
// warning; loft#829 finished the job, because a warning is not an answer:
// the call still returned "", which a caller cannot tell from an empty file.
// It now returns null, and the warning — which lived in the interpreter's
// read alone — is emitted by the one read both backends share.

/// P166: reading a non-UTF-8 file via .content() must emit a stderr warning
/// containing the phrase "non-UTF-8 bytes" along with the file size and a
/// pointer at the binary-read idiom — on BOTH backends (loft#829: the warning
/// was interpreter-only, so `--native` read binary in silence).
#[test]
fn p166_content_on_binary_file_warns() {
    for backend in ["--interpret", "--native"] {
        let dir = std::env::temp_dir();
        let tag = backend.trim_start_matches('-');
        let bin_path = dir.join(format!("loft_p166_binary_{tag}.bin"));
        // Non-UTF-8 bytes: 0xFF and 0xFE are invalid UTF-8 start bytes.
        std::fs::write(&bin_path, [0xFFu8, 0xFE, 0xFD, 0xFC, 0xFB])
            .expect("write temp binary file");

        let script_path = dir.join(format!("loft_p166_script_{tag}.loft"));
        // Use forward slashes in the embedded path so the loft lexer doesn't
        // treat Windows backslashes as escape sequences (`\U`, `\R`, …).
        let path_in_script = bin_path.display().to_string().replace('\\', "/");
        let script = format!(
            "fn main() {{\n  \
                f = file(\"{path_in_script}\");\n  \
                c = f.content();\n  \
                println(\"null={{c == null}}\");\n  \
                assert(c == null, \"binary content() answers null, not empty text\");\n\
             }}\n"
        );
        std::fs::write(&script_path, &script).expect("write temp script");

        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(&script_path)
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let _ = std::fs::remove_file(&bin_path);
        let _ = std::fs::remove_file(&script_path);

        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{backend}: program should exit 0 (null is a valid answer); \
             stdout={stdout:?} stderr={stderr:?}"
        );
        assert!(
            stderr.contains("non-UTF-8 bytes"),
            "{backend}: expected 'non-UTF-8 bytes' warning in stderr; got stderr={stderr:?}"
        );
        assert!(
            stderr.contains("5 bytes in file"),
            "{backend}: warning should include the actual file size; got stderr={stderr:?}"
        );
        assert!(
            stderr.contains("#format = LittleEndian"),
            "{backend}: warning should name the binary-read idiom; got stderr={stderr:?}"
        );
        assert!(
            stderr.contains("read_bytes(path)"),
            "{backend}: warning should name the exact byte reader; got stderr={stderr:?}"
        );
    }
}

// ── P168: arguments() leaked argv when zero script-level args ────────────
//
// Prior to 2026-04-17, `src/database/format.rs::os_arguments` fell back
// to `std::env::args_os()` when `user_args` was empty, returning the
// binary path + loft CLI flags + script path.  P131's filter only ran
// through the `user_args` path.  Fix: always return `user_args`
// (an empty vector is a correct result).

/// P168: running a loft script with no script-level args must produce
/// `arguments()` == [] — no binary path, no `--interpret`, no script path.
#[test]
fn p168_arguments_empty_when_no_script_args() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p168_args_empty.loft");
    // Script prints each argument; empty vector → no lines, just "count=0".
    std::fs::write(
        &path,
        "fn main() {\n  \
             a = arguments();\n  \
             println(\"count={len(a)}\");\n  \
             for s in a { println(\"  [{s#index}] {s}\"); }\n\
         }\n",
    )
    .expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "expected exit 0; stderr={stderr:?}");
    assert!(
        stdout.contains("count=0"),
        "arguments() should be empty when no script args given; got stdout={stdout:?}"
    );
    // Belt-and-suspenders: make sure the binary path isn't smuggled in.
    assert!(
        !stdout.contains("target/release/loft"),
        "arguments() must not leak the loft binary path; got stdout={stdout:?}"
    );
    assert!(
        !stdout.contains("--interpret"),
        "arguments() must not leak loft CLI flags; got stdout={stdout:?}"
    );
}

// ── P169: lambda-suggestion error message accuracy ───────────────────────
//
// The `|x: T| { ... }` form is rejected ("Type annotations are not
// allowed in |x| lambdas").  The suggested alternative used to include
// `-> <ret>` in the template, misleading users to try `-> void` which
// fails with "Undefined type void" — loft omits the `->` clause for
// void returns.  Fix: updated the suggestion in
// `src/parser/vectors.rs` to make `<ret>` optional and explicitly
// call out `-> void` as invalid.

/// P169: the "Type annotations not allowed in |x|" diagnostic must
/// suggest `fn(x: <type>) { ... }` (no mandatory `-> <ret>`) and warn
/// that `-> void` is not a valid type.
#[test]
fn p169_lambda_suggestion_mentions_omitting_return_type() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p169_lambda_types.loft");
    std::fs::write(&path, "fn main() {\n  _ = |x: integer| { x * 2 };\n}\n")
        .expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    assert!(!out.status.success(), "expected parse error");
    // @P282: loft emits parse diagnostics to STDERR (rustc / clang convention).
    let stdout = String::from_utf8_lossy(&out.stderr);
    // The new suggestion shows `fn(x: <type>) { ... }` without mandatory
    // `-> <ret>`, and calls out `-> void` as invalid.
    assert!(
        stdout.contains("fn(x: <type>) { ... }"),
        "suggestion should be `fn(x: <type>) {{ ... }}`; got stderr={stdout:?}"
    );
    assert!(
        stdout.contains("`-> void` is not a valid type"),
        "suggestion should warn about `-> void`; got stderr={stdout:?}"
    );
}

// ── 6a.18: moros_glb CLI tool end-to-end ──────────────────────────────────

/// Phase 6a.18 — the `moros_glb` CLI example reads a map JSON and writes
/// a GLB.  This verifies the full loft-level pipeline: JSON parse → Map →
/// build_hex_meshes → save_scene_glb, driven from a standalone script via
/// `arguments()`.
///
/// Exercises the MIXED path (interpreted entry package calling auto-native
/// library functions like `save_scene_glb`), which is the default for a
/// `--interpret` run after #460.  This is the exact path that surfaced #461:
/// an auto-native cdylib cached in one consumer's context baked type-table
/// indices that were wrong in another's, so `OpWriteFile` wrote 8-byte fields
/// for `as i32` (GLB version 0, +28 bytes).  A correct version-2 header proves
/// the #461 fix (context-keyed cdylib freshness) holds end to end.
#[test]
fn moros_glb_cli_end_to_end() {
    let dir = std::env::temp_dir();
    let json_path = dir.join("loft_moros_glb_input.json");
    let glb_path = dir.join("loft_moros_glb_output.glb");
    // Minimal map with one material in the palette.
    let map_json = r#"{
        "m_name": "cli_test",
        "m_chunks": [],
        "m_material_palette": [
            {"md_name": "stone", "md_category": "terrain", "md_stair_kind": "",
             "md_texture": 0, "md_tint_r": 120, "md_tint_g": 120, "md_tint_b": 120,
             "md_walkable": 1, "md_swimmable": 0, "md_climbable": 0,
             "md_slippery": 0, "md_loud": 0}
        ],
        "m_wall_palette": [],
        "m_item_palette": [],
        "m_spawns": [],
        "m_routines": []
    }"#;
    std::fs::write(&json_path, map_json).expect("write map JSON");
    let _ = std::fs::remove_file(&glb_path);

    let script = workspace_root().join("tests/fixtures/libs/moros_render/examples/moros_glb.loft");
    let path_flag = format!("{}/", workspace_root().display());
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--path")
        .arg(&path_flag)
        .arg(&script)
        .arg(&json_path)
        .arg(&glb_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "CLI should exit 0; stdout={stdout:?}; stderr={stderr:?}"
    );
    assert!(
        stdout.contains("wrote"),
        "CLI should print 'wrote <path>'; got stdout={stdout:?}"
    );
    assert!(
        glb_path.exists(),
        "GLB file should be written at {}",
        glb_path.display()
    );
    // Read the first 4 bytes and verify 'glTF' magic (LE bytes).
    let bytes = std::fs::read(&glb_path).expect("read GLB");
    let _ = std::fs::remove_file(&json_path);
    let _ = std::fs::remove_file(&glb_path);
    assert!(
        bytes.len() >= 12,
        "GLB should have at least the 12-byte header; got {} bytes",
        bytes.len()
    );
    assert_eq!(&bytes[0..4], b"glTF", "GLB should start with 'glTF' magic");
    // Version is bytes 4..8, little-endian u32; must be 2.
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    assert_eq!(version, 2, "GLB version should be 2");
}

/// P166: reading a valid UTF-8 text file via .content() must NOT emit the
/// warning — the signal is strictly on decode failure, not on all binary
/// opens.
#[test]
fn p166_content_on_text_file_no_warning() {
    let dir = std::env::temp_dir();
    let text_path = dir.join("loft_p166_text.txt");
    std::fs::write(&text_path, "hello world\n").expect("write temp text file");

    let script_path = dir.join("loft_p166_text_script.loft");
    // Forward slashes so Windows backslashes don't become lexer escapes.
    let path_in_script = text_path.display().to_string().replace('\\', "/");
    let script = format!(
        "fn main() {{\n  \
            f = file(\"{path_in_script}\");\n  \
            c = f.content();\n  \
            assert(len(c) > 0, \"content should be non-empty\");\n\
         }}\n"
    );
    std::fs::write(&script_path, &script).expect("write temp script");

    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&text_path);
    let _ = std::fs::remove_file(&script_path);

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "text-file read should succeed");
    assert!(
        !stderr.contains("non-UTF-8 bytes"),
        "text file should not trigger the P166 warning; got stderr={stderr:?}"
    );
}

/// DX-source-map — the native-codegen emitter writes
/// `// loft:<file>:<line>` comments above each function header and
/// each statement so rustc errors on the generated Rust code map
/// back to the originating loft source.
#[test]
fn native_emit_includes_loft_source_map() {
    let dir = std::env::temp_dir();
    let script_path = dir.join("loft_source_map_demo.loft");
    let script = "fn add(a: integer, b: integer) -> integer { a + b }\n\
                  fn main() { x = add(1, 2); println(\"{x}\") }\n";
    std::fs::write(&script_path, script).expect("write temp script");

    let out = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-rust")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "introspect should succeed");
    // Loft's source-map emission canonicalizes paths.  On Windows
    // canonicalize() returns the `\\?\` UNC form; on Linux/macOS
    // it returns the absolute path with symlinks resolved.  Match
    // the test's expectation against the same canonical form.  A
    // simple `.display()` form fails on Windows because the test's
    // path lacks the UNC prefix.
    let canonical = std::fs::canonicalize(&script_path).unwrap_or_else(|_| script_path.clone());
    let path_str = canonical.display().to_string();
    // Function-header comment maps to the .loft source line.
    // Use ends_with-style match (`// loft:{stem-suffix}:1\nfn n_add(`)
    // when the full path comparison fails — robust to canonical
    // path variations across platforms.
    let header_n_add = format!("// loft:{path_str}:1\nfn n_add(");
    let header_n_main = format!("// loft:{path_str}:2\nfn n_main(");
    let stem_n_add = "loft_source_map_demo.loft:1\nfn n_add(".to_string();
    let stem_n_main = "loft_source_map_demo.loft:2\nfn n_main(".to_string();
    assert!(
        stdout.contains(&header_n_add) || stdout.contains(&stem_n_add),
        "expected source-map header above n_add (canonical or stem match); got {stdout}"
    );
    assert!(
        stdout.contains(&header_n_main) || stdout.contains(&stem_n_main),
        "expected source-map header above n_main (canonical or stem match); got {stdout}"
    );
}

/// P196: tuple struct field whose element is a fn-ref must project
/// `.0` from the runtime `(u32, DbRef)` tuple before the OpSetInt4
/// `as i32` cast.  Regression guard: a Var-of-fn-ref-tuple source
/// (which can't be folded to `Value::Int(d_nr)` at parse time)
/// must emit `(i64::from((var.0).0))` — i.e. project u32 d_nr from
/// the tuple-element's `(u32, DbRef)` shape and widen for the
/// template's null-check.  Without the fix the codegen substitutes
/// `var.0 as i32` directly, which rustc rejects with E0605 (non-
/// primitive cast on tuple type) and E0308 on the matching null
/// check `var.0 == i64::MIN`.
#[test]
fn p196_native_codegen_projects_fn_ref_d_nr() {
    let dir = std::env::temp_dir();
    let script_path = dir.join("loft_p196_codegen.loft");
    let script = "struct Pair { v: (fn(integer) -> integer, integer) }\n\
                  fn p_dbl(x: integer) -> integer { x + x }\n\
                  fn build(f: fn(integer) -> integer, n: integer) -> (fn(integer) -> integer, integer) { (f, n) }\n\
                  fn main() { pp = build(p_dbl, 21); p = Pair { v: pp }; }\n";
    std::fs::write(&script_path, script).expect("write temp script");

    let out = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-rust")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "introspect should succeed");
    // Fix invariant: every set_i32_raw emitted for the fn-ref tuple
    // element widens via `i64::from(...)` of the projected `.0` —
    // not a bare `var.0 as i32` (which rustc rejects on tuple type).
    // The tuple is read where it lives (loft#1594: `pp` itself, no
    // `__ref_1` copy of it), so the projection names `var_pp`.
    assert!(
        stdout.contains("i64::from((var_pp.0).0)"),
        "expected fn-ref d_nr projection `i64::from((var_pp.0).0)`; got:\n{stdout}"
    );
    // And the buggy bare `var_pp.0 == i64::MIN` shape must be gone —
    // it would compare a `(u32, DbRef)` tuple to an i64.
    assert!(
        !stdout.contains("(var_pp.0) == i64::MIN"),
        "fn-ref tuple field should not be compared to i64::MIN as a bare tuple; got:\n{stdout}"
    );
}

/// DX-diff — `--introspect --diff <baseline>` exits 0 when the
/// baseline matches and 1 when it differs (mirroring `diff -u`'s
/// exit code).  Lets devs answer "did my parser tweak change
/// anything?" with a single command.
#[test]
fn introspect_diff_against_baseline() {
    let dir = std::env::temp_dir();
    let script_path = dir.join("loft_diff_demo.loft");
    let baseline_path = dir.join("loft_diff_baseline.txt");
    let script = "fn main() { println(\"hello\") }\n";
    std::fs::write(&script_path, script).expect("write temp script");

    // Capture baseline.
    let baseline_out = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-types")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("baseline capture failed");
    std::fs::write(&baseline_path, &baseline_out.stdout).expect("write baseline");

    // Identical inputs → exit 0.
    let same = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-types")
        .arg("--diff")
        .arg(&baseline_path)
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("diff (identical) failed");
    assert_eq!(
        same.status.code(),
        Some(0),
        "identical inputs should exit 0; stderr={:?}",
        String::from_utf8_lossy(&same.stderr)
    );

    // Mutate the script with a STRUCTURAL change so the types table
    // differs (string-literal changes alone don't show up in
    // `--show-types`).
    std::fs::write(
        &script_path,
        "fn add(a: integer) -> integer { a + 1 }\nfn main() { println(\"hello\") }\n",
    )
    .expect("rewrite temp script");

    let differs = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-types")
        .arg("--diff")
        .arg(&baseline_path)
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("diff (differs) failed");
    assert_eq!(
        differs.status.code(),
        Some(1),
        "differing inputs should exit 1; stdout={:?}",
        String::from_utf8_lossy(&differs.stdout)
    );

    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&baseline_path);
}

/// `--show-types --trace` emits a per-expression type tape that
/// makes dep-propagation flow visible at every chaining step
/// (`.field`, `.tuple_idx`, `[idx]`, `(args)`).  Designed so a
/// future P197-class bug shows up as a missing `[host]` suffix
/// on an intermediate type, not just the eventual return.
#[test]
fn introspect_show_types_trace_renders_per_expression() {
    let dir = std::env::temp_dir();
    let script_path = dir.join("loft_trace_demo.loft");
    let script = "struct A { v: (text, text) }\n\
                  fn first() -> text {\n  \
                      a = A { v: (\"hello\", \"world\") };\n  \
                      a.v.0\n\
                  }\n\
                  fn main() { println(\"{first()}\") }\n";
    std::fs::write(&script_path, script).expect("write temp script");

    let out = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-types")
        .arg("--trace")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "introspect should succeed");
    assert!(
        stdout.contains("trace (per-expression types):"),
        "expected trace section header; got {stdout}"
    );
    // The per-step tape shows the tuple's element types each carry
    // the host's dep AFTER the `.v` step — this is the line that
    // would have read `(text, text)` (no `[a]`) before the P197 fix.
    assert!(
        stdout.contains("(text[\"a\"], text[\"a\"])"),
        "expected `a.v` step to render `(text[\"a\"], text[\"a\"])` \
         (each tuple element carries dep on host `a`); got {stdout}"
    );
    // And the final `.0` extraction preserves the dep.
    assert!(
        stdout.contains("text[\"a\"]"),
        "expected final `.0` step to render `text[\"a\"]`; got {stdout}"
    );
}

/// Plan-08 phase 01 — `--introspect --show-types` emits a per-fn
/// type table where `Type::show()` includes dependency suffixes
/// (e.g. `text["a"]`).  Designed to surface dep-tracking bugs at a
/// glance; the post-P197 fix means a tuple-element text returned
/// from a struct field carries the host as a dep.  This test pins
/// the visible `text["a"]` annotation so any regression in dep
/// propagation through `Type::Tuple` shows up here too.
#[test]
fn introspect_show_types_renders_deps() {
    let dir = std::env::temp_dir();
    let script_path = dir.join("loft_introspect_types_demo.loft");
    let script = "struct A { v: (text, text) }\n\
                  fn first() -> text {\n  \
                      a = A { v: (\"hello\", \"world\") };\n  \
                      a.v.0\n\
                  }\n\
                  fn main() { println(\"{first()}\") }\n";
    std::fs::write(&script_path, script).expect("write temp script");

    let out = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-types")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "introspect should succeed");
    assert!(
        stdout.contains("=== types ==="),
        "expected types section header; got {stdout}"
    );
    // The fix for P197 propagates the host (`a`) as a dep through
    // tuple-element text reads to the function's return type.
    // If this assertion fails, the dep propagation in
    // `Type::depending` / `parse_part` regressed.
    assert!(
        stdout.contains("n_first -> text[\"a\"]"),
        "expected `n_first -> text[\"a\"]` (P197 dep propagation); got {stdout}"
    );
}

/// @P367 regression: `loft --tests` must report a test FAILED (and exit
/// non-zero) when an `assert(false)` / `panic` / divide-by-zero fired inside it
/// — these set a typed runtime fault and halt WITHOUT a Rust panic (the C66
/// path), so the runner previously scored them PASSED.
#[test]
fn tests_runner_fails_on_assert_and_fault() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p367_fault.loft");
    std::fs::write(
        &path,
        "fn test_bad_assert() { assert(false, \"boom\"); }\n\
         fn test_panic() { panic(\"kapow\"); }\n\
         fn test_ok() { assert(1 == 1, \"fine\"); }\n",
    )
    .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--tests")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success(),
        "expected non-zero exit when a test asserts/panics; stdout={stdout}"
    );
    assert!(
        stdout.contains("FAILED") && stdout.contains("2 failed") && stdout.contains("1 passed"),
        "expected 2 failed / 1 passed; got {stdout}"
    );
    assert!(
        stdout.contains("assertion failed: boom") && stdout.contains("panic: kapow"),
        "expected the fault messages in the FAIL lines; got {stdout}"
    );
}

/// @P367 companion: a `@EXPECT_FAIL` test whose intentional fault fires must
/// still PASS (and the file exit 0) — the fix must not break expected-fail.
#[test]
fn tests_runner_expect_fail_still_passes() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p367_expectfail.loft");
    std::fs::write(
        &path,
        "// @EXPECT_FAIL: boom\nfn test_intentional() { assert(false, \"boom\"); }\n",
    )
    .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--tests")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "expected exit 0 for an @EXPECT_FAIL intentional fault; stdout={stdout}"
    );
    assert!(
        stdout.contains("1 passed"),
        "expected the @EXPECT_FAIL test to pass; got {stdout}"
    );
}

/// @P368 regression: the divide-by-zero warning must NOT fire when the divisor
/// is a non-zero literal constant (int OR float), but MUST still fire for a
/// variable divisor.  Also: the message must not say "integer division".
///
/// @P368 follow-up (dryopea-surfaced): the warning must ALSO not fire when
/// the dividend is float / single and the divisor is an integer literal
/// (`x / 3` with `x: float`).  The parser wraps the literal in an
/// `OpConvFloatFromInt` cast for type matching; without seeing through that
/// cast, `lit_nonzero` returns None and the warning fires spuriously.  The
/// `e = x / 3` case in this test exercises that arm.
#[test]
fn div_by_literal_constant_no_warning() {
    let dir = std::env::temp_dir();
    let safe = dir.join("loft_p368_safe.loft");
    std::fs::write(
        &safe,
        "fn calc(x: float, c: integer) -> float {\n  \
           a = x / 2.0;\n  b = x / 0.75;\n  d = c / 2;\n  \
           e = x / 3;\n  \
           x + a + b + (d as float) + e\n}\n\
         fn main() { println(\"{calc(10.0, 10)}\"); }\n",
    )
    .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&safe)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("may produce null on divide-by-zero"),
        "literal-constant divisors must not warn; got stderr={stderr}"
    );

    // @PLN25 DN3 — a variable divisor no longer WARNS: `c / y` now TYPES `integer?`,
    // so the null lives in the type and `(N-Store)` is the enforcement (the runtime-null
    // warning is retired for Div/Rem under DN1). An inferred local absorbs the nullability,
    // so an undefended division compiles and runs with no warning. (Storing it into a
    // non-null slot is a hard `(N-Store)` error, covered by 102-expected-errors.)
    let unsafe_ = dir.join("loft_p368_unsafe.loft");
    std::fs::write(
        &unsafe_,
        "fn main() { c = 10; y = 2; d = c / y; println(\"{d}\"); }\n",
    )
    .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&unsafe_)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("division may produce null"),
        "the div warning is retired under DN1 (the type carries the null); got stderr={stderr}"
    );
}

// ── #333 / C80: undefended div-by-zero is null-and-continue on BOTH backends ──
// E-Uncomp (formal/operational.md): a calculation fault never halts.  `5 / z`
// with `z == 0` yields the null sentinel and execution CONTINUES (exit 0) — the
// interpreter and the compiled native binary must agree.  (Was: both exited 1
// via the raise/NATIVE_FAIL_FAST halt; reversed by C80.)
#[test]
fn issue_333_div_zero_null_continues() {
    let pid = std::process::id();
    let script = std::env::temp_dir().join(format!("loft_i333_{pid}.loft"));
    std::fs::write(
        &script,
        "fn main() {\n  z = 0;\n  a = 5 / z;\n  print(\"reached a={a}\");\n}\n",
    )
    .expect("write script");
    for mode in ["--interpret", "--native"] {
        let out = Command::new(loft_bin())
            .arg(mode)
            .arg(&script)
            .current_dir(workspace_root())
            .output()
            .expect("invoke loft");
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{mode}: expected exit 0 (null-and-continue), got {:?}\nstdout: {stdout}\nstderr: {stderr}",
            out.status.code()
        );
        assert!(
            stdout.contains("reached a=null"),
            "{mode}: execution must continue past the fault with null: {stdout}"
        );
    }
    let _ = std::fs::remove_file(&script);
}

/// loft#1012 — `verify-self` must NOT exit 0 when it verified nothing.
///
/// The message was always honest ("not a release bundle — nothing to check against"); the
/// exit code is what gets read, and `loft verify-self && deploy` was green on an install the
/// command could not examine. *Verified intact* and *could not verify anything* are the two
/// answers a caller most needs to tell apart, and they were the same answer.
///
/// A test-run binary lives in `target/release/`, whose bundle root has no `SHA256SUMS` — so
/// this is exactly the unverifiable case, and it is the one the CLI can reach without
/// building a release bundle. The verified (0) and mismatched (1) paths need a real bundle
/// and are covered where one exists (`verify_self`'s own tests over `local_checks`).
#[test]
fn verify_self_exits_two_when_it_verified_nothing() {
    let out = Command::new(loft_bin())
        .arg("verify-self")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The precondition: this really is the "nothing to check" case, not a passing verify.
    assert!(
        stdout.contains("not a release bundle"),
        "expected the unverifiable case for a dev-tree binary; got {stdout:?}"
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "`verify-self` that checked nothing must not report success (loft#1012); got {:?}, \
         stdout={stdout:?}",
        out.status.code()
    );
    assert!(
        !out.status.success(),
        "`loft verify-self && deploy` must not proceed on an install it could not examine"
    );
}

// loft#1289 — a reader that stops reading ends a pipeline; it does not fault the writer.
//
// `print!` panics on any write error, so `prog | head` ended in a Rust panic naming a `std`
// source line, and when stderr shared the closed pipe the panic PRINTER failed too and the
// process ABORTED — leaving `.loft/loft-crash-<pid>.txt` blaming a line of
// `default/01_code.loft` that has nothing wrong with it.  Both symptoms are one cause.
//
// These run the BINARY through a real pipe, because that is the only place the fault exists:
// the write only fails once an OS pipe has a closed read end, and no library-level harness
// has one.

/// A `.loft` file in a scratch directory of its own, so the `.loft/` cache a run may create
/// cannot collide with another test's.
fn epipe_fixture(name: &str, body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("loft-epipe-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let script = dir.join(format!("{name}.loft"));
    std::fs::write(&script, body).expect("write fixture");
    (dir, script)
}

/// More output than a pipe buffer holds (~64 KiB), so a write really does meet the closed
/// read end instead of landing in the buffer while the child runs to completion.
const EPIPE_MANY: &str = "fn main() { for i in 0..20000 { println(\"line {i} padded to make the pipe buffer overflow\"); } }\n";

/// Enough output to outlive the reader, and the reader takes three lines.
///
/// The child's stdout handle is dropped after the third line, which closes the read end —
/// the same thing `| head -3` does, without needing a shell.
#[cfg(unix)]
fn run_until_reader_leaves(script: &std::path::Path, dir: &std::path::Path, mode: &str) -> String {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let mut child = Command::new(loft_bin())
        .arg(mode)
        .arg(script)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to invoke loft binary");
    {
        let out = child.stdout.take().expect("piped stdout");
        let mut reader = BufReader::new(out);
        let mut line = String::new();
        for _ in 0..3 {
            line.clear();
            let _ = reader.read_line(&mut line);
        }
        // Dropping the reader closes this end of the pipe — from here the child's next
        // write gets EPIPE.
    }
    let out = child.wait_with_output().expect("wait");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "a closed stdout is the ordinary end of `prog | head`, not a fault: got {:?} \
         on {mode}; stderr={stderr:?}",
        out.status.code()
    );
    assert!(
        !stderr.contains("panicked"),
        "the writer must not panic when its reader leaves ({mode}); stderr={stderr:?}"
    );
    stderr
}

#[test]
#[cfg(unix)]
fn a_reader_that_stops_reading_does_not_fault_the_interpreter() {
    let (dir, script) = epipe_fixture("many-interp", EPIPE_MANY);
    run_until_reader_leaves(&script, &dir, "--interpret");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[cfg(unix)]
fn a_reader_that_stops_reading_does_not_fault_the_native_backend() {
    let (dir, script) = epipe_fixture("many-native", EPIPE_MANY);
    run_until_reader_leaves(&script, &dir, "--native");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Run `script` with stdout AND stderr on ONE pipe, read `keep` lines, then close the read
/// end — `prog 2>&1 | head -N` without a shell.
///
/// Built from `libc::pipe` rather than `sh -c … PIPESTATUS`: `PIPESTATUS` is a bash array
/// and `/bin/sh` here is dash, where it expands to nothing — the first version of this test
/// compared that empty string against "134" and passed while measuring nothing.
#[cfg(unix)]
fn run_with_merged_pipe(
    script: &std::path::Path,
    dir: &std::path::Path,
    keep: usize,
) -> std::process::ExitStatus {
    use std::io::{BufRead, BufReader};
    use std::os::fd::{FromRawFd, OwnedFd};
    let mut fds = [0 as libc::c_int; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe");
    // CLOSE-ON-EXEC on BOTH originals.  `Command` DUPS them onto the child's 0/1/2, and
    // without this the originals are inherited too — so the child holds its own READ end,
    // the pipe never reports a closed reader, and once the buffer fills the child blocks
    // forever.  Measured: the first version of this helper hung for twenty minutes in
    // `anon_pipe_write` with `fd 3 -> pipe` in its own `/proc/<pid>/fd`.
    for fd in fds {
        assert_ne!(
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
            -1,
            "FD_CLOEXEC"
        );
    }
    // SAFETY: both ends come from a successful `pipe` and are owned from here on.
    let (read_end, write_end) =
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let write_dup = write_end.try_clone().expect("dup the write end");
    let mut child = Command::new(loft_bin())
        .arg("--interpret")
        .arg(script)
        .current_dir(dir)
        .stdout(Stdio::from(write_end))
        .stderr(Stdio::from(write_dup))
        .spawn()
        .expect("failed to invoke loft binary");
    {
        let mut reader = BufReader::new(std::fs::File::from(read_end));
        let mut line = String::new();
        for _ in 0..keep {
            line.clear();
            let _ = reader.read_line(&mut line);
        }
        // Dropping the reader closes the last read end; the child's next write meets EPIPE.
    }
    child.wait().expect("wait")
}

/// The worse half: stderr sharing the closed pipe turned the panic into a SIGABRT, and the
/// crash reporter then wrote a report naming a stdlib line.
///
/// Verified to FIRE by injection (the `BrokenPipe` arm disabled): `status=ExitStatus(
/// unix_wait_status(134))`, signal 6, which is the filed symptom exactly.
#[test]
#[cfg(unix)]
fn stderr_sharing_a_closed_pipe_does_not_abort_or_write_a_crash_report() {
    use std::os::unix::process::ExitStatusExt;
    // A `never-read` warning goes to stderr BEFORE the program runs, so one line is
    // satisfied by the diagnostic and every later write — diagnostic or program output —
    // meets the closed pipe.
    let (dir, script) = epipe_fixture(
        "warn",
        // Far more than a pipe buffer holds (~64 KiB): with too little output the child
        // writes everything into the buffer and EXITS before the reader closes, so no write
        // ever meets a closed pipe and the test measures nothing.  Verified by injection —
        // at 500 lines this cell passed with the cure removed.
        "fn main() { unused = 1; for i in 0..20000 { println(\"line {i} padded to make the pipe buffer overflow\"); } }\n",
    );
    // `.loft/` must EXIST or the reporter has nowhere to write and the test would pass
    // for the wrong reason.
    std::fs::create_dir_all(dir.join(".loft")).expect("cache dir");
    let status = run_with_merged_pipe(&script, &dir, 1);
    assert_eq!(
        status.signal(),
        None,
        "`prog 2>&1 | head` must not die by signal (SIGABRT was 6); status={status:?}"
    );
    assert!(
        status.success(),
        "a closed pipe is the ordinary end of the run; got {:?}",
        status.code()
    );
    let reports: Vec<_> = std::fs::read_dir(dir.join(".loft"))
        .expect("read cache dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("loft-crash-"))
        .collect();
    assert!(
        reports.is_empty(),
        "a broken pipe is not a crash and must leave no report; found {reports:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The CONTROL that keeps the cure honest: a write error that is NOT a broken pipe is a real
/// fault and stays loud.  Exiting 0 on every failed write would pass every test above.
#[test]
#[cfg(unix)]
fn a_full_disk_is_still_a_failure() {
    if !std::path::Path::new("/dev/full").exists() {
        return;
    }
    let (dir, script) = epipe_fixture("full", EPIPE_MANY);
    let full = std::fs::File::create("/dev/full").expect("open /dev/full");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script)
        .current_dir(&dir)
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .output()
        .expect("failed to invoke loft binary");
    assert!(
        !out.status.success(),
        "ENOSPC on stdout is a genuine failure and must not be reported as success"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The second CONTROL: a program that FAILS still reports its failure through a pipe the
/// reader left early.  The broken-pipe exit must not mask a compile error.
#[test]
#[cfg(unix)]
fn a_compile_error_still_exits_nonzero_through_a_closed_pipe() {
    let (dir, script) = epipe_fixture("bad", "fn main() { qqq(); }\n");
    let status = run_with_merged_pipe(&script, &dir, 1);
    assert_eq!(
        status.code(),
        Some(1),
        "a program that does not compile still fails, whatever the reader did; got {status:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
