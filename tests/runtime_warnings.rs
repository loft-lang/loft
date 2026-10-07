// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-07 phase 4e.2 — undefended fault-site compile-time warning.
//!
//! Binary-level tests: invoke the loft compiler with `LOFT_NO_WARN_RUNTIME`
//! explicitly UNset (the in-process test harness sets it; here we bypass
//! that suppression because the WHOLE POINT is to assert on the
//! warning's presence/absence) and inspect the stderr output.
//!
//! Coverage:
//!   - The warning fires for every undefended fault site
//!     (`undefended_*` cells).
//!   - The warning is silenced by each of the four canonical safe
//!     patterns from the design's "Easy-proof skip list — REQUIRED"
//!     (`skip_*` cells).
//!   - The warning is silenced when 4d.1 / 4d.2 / 4e.1 swap fires
//!     (`defended_*` cells — covered alongside the runtime tests in
//!     `tests/runtime_logging.rs`; rechecked here for the compile-time
//!     half of each defense).
//!   - `LOFT_NO_WARN_RUNTIME=1` silences the warning entirely
//!     (`silenced_by_env` cell).

use loft::file_access as fa;
use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run a loft snippet under `--interpret` with `LOFT_NO_WARN_RUNTIME`
/// explicitly UNset and return (stdout, stderr, exit-status-code).
fn run_with_warnings(name: &str, source: &str) -> (String, String, Option<i32>) {
    let script_path = std::env::temp_dir().join(format!("loft_w42_{name}.loft"));
    fa::write(&script_path, source).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .current_dir(workspace_root())
        .env_remove("LOFT_NO_WARN_RUNTIME")
        .output()
        .expect("failed to invoke loft binary");
    let _ = fa::remove_file(&script_path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// @PLN87 P3 (W4) — the redundant-`&` lint is ON by default (P3.2 flip); a redundant
/// `&`-reference is double-indirect and materially slower, so users see it without
/// opting in. `LOFT_NO_WARN_RUNTIME=1` silences it (the runtime-warning family switch).
fn run_with_w4(name: &str, source: &str) -> (String, String, Option<i32>) {
    run_with_w4_env(name, source, false)
}

/// Same, with `--explain` when asked — @PLN131 moved this lint's "unless you REASSIGN"
/// clause into its fix's condition, so a test that pins it has to look there.
fn run_with_w4_env(name: &str, source: &str, explain: bool) -> (String, String, Option<i32>) {
    let script_path = std::env::temp_dir().join(format!("loft_w4_{name}.loft"));
    fa::write(&script_path, source).expect("write temp script");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret");
    if explain {
        cmd.arg("--explain");
    }
    let out = cmd
        .arg(&script_path)
        .current_dir(workspace_root())
        .env("LOFT_WARN_REDUNDANT_AMP", "1")
        .output()
        .expect("failed to invoke loft binary");
    let _ = fa::remove_file(&script_path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

// ── Warning fires on every undefended fault site ────────────────────────────

// @PLN25 DN3 — `/` and `%` now TYPE `integer?` (the null is in the type and `(N-Store)`
// forces discharge at stores), so the redundant runtime-null warning is RETIRED under DN1
// (operators.rs::emit_undefended_warning early-returns for Div/Rem). An inferred local
// absorbs the nullability, so the division still runs. These guard that the retirement
// holds — the index warnings below stay until the index flip lands.
#[test]
fn div_by_var_no_warn_retired_dn3() {
    let source = "\
fn main() {
  z = 5;
  x = 10 / z;
  print(\"x={x}\\n\");
}
";
    let (stdout, diag, _code) = run_with_warnings("undef_div", source);
    assert!(
        !diag.contains("division may produce null"),
        "div warning is retired under DN1 (the type carries the null); got {diag:?}"
    );
    assert!(
        stdout.contains("x=2"),
        "the division still runs (10/5=2); got {stdout:?}"
    );
}

#[test]
fn mod_by_var_no_warn_retired_dn3() {
    let source = "\
fn main() {
  z = 5;
  x = 10 % z;
  print(\"x={x}\\n\");
}
";
    let (stdout, diag, _code) = run_with_warnings("undef_mod", source);
    assert!(
        !diag.contains("modulus may produce null"),
        "mod warning is retired under DN1 (the type carries the null); got {diag:?}"
    );
    assert!(
        stdout.contains("x=0"),
        "the modulus still runs (10%5=0); got {stdout:?}"
    );
}

// @PLN25 index flip — `v[i]` / `s[i]` now TYPE `τ?` (the OOB null is in the type and
// `(N-Store)` forces discharge at stores), so the redundant runtime-null warning is RETIRED
// under DN1 (operators.rs::emit_undefended_warning early-returns for VectorIndex/TextIndex).
// An inferred local absorbs the nullability, so the undefended index still runs. These guard
// that the retirement holds (constant / iter-var / guard fit-proofs stay non-null below).
#[test]
fn vec_index_by_var_no_warn_retired() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  i = 1;
  x = v[i];
  print(\"x={x}\\n\");
}
";
    let (stdout, diag, _code) = run_with_warnings("undef_vec", source);
    assert!(
        !diag.contains("warning: `v[i]` may produce null"),
        "the v[i] warning is retired under DN1 (the type carries the null); got {diag:?}"
    );
    assert!(
        stdout.contains("x=20"),
        "the index still reads v[1]=20; got {stdout:?}"
    );
}

#[test]
fn text_index_by_var_no_warn_retired() {
    let source = "\
fn main() {
  s = \"abc\";
  i = 1;
  c = s[i];
  print(\"c={c}\\n\");
}
";
    let (stdout, diag, _code) = run_with_warnings("undef_text", source);
    assert!(
        !diag.contains("warning: `s[i]` may produce null"),
        "the s[i] warning is retired under DN1 (the type carries the null); got {diag:?}"
    );
    assert!(
        stdout.contains("c=b"),
        "the index still reads s[1]='b'; got {stdout:?}"
    );
}

// ── Skip pattern 1 — constant non-zero literal divisor ──────────────────────

#[test]
fn skip_constant_nonzero_divisor() {
    let source = "\
fn main() {
  x = 10 / 3;
  y = 10 % 7;
  print(\"x={x} y={y}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("skip_const_div", source);
    assert!(
        !diag.contains("warning: division may produce null"),
        "constant non-zero divisor must NOT warn; got stdout={diag:?}"
    );
    assert!(
        !diag.contains("warning: modulus may produce null"),
        "constant non-zero modulus must NOT warn; got stdout={diag:?}"
    );
}

// ── Skip pattern 2 — constant non-negative literal index ────────────────────

#[test]
fn skip_constant_index() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  s = \"abc\";
  print(\"v[1]={v[1]} s[0]={s[0]}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("skip_const_idx", source);
    assert!(
        !diag.contains("warning: `v[i]` may produce null"),
        "constant index must NOT warn; got stdout={diag:?}"
    );
    assert!(
        !diag.contains("warning: `s[i]` may produce null"),
        "constant text index must NOT warn; got stdout={diag:?}"
    );
}

// ── Skip pattern 3 — index is a for-loop iteration variable ─────────────────

#[test]
fn skip_for_loop_iter_var() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  for i in 0..len(v) {
    x = v[i];
    print(\"x={x}\\n\");
  }
}
";
    let (_stdout, diag, _code) = run_with_warnings("skip_for_iter", source);
    assert!(
        !diag.contains("warning: `v[i]` may produce null"),
        "for-loop iter var index must NOT warn; got stdout={diag:?}"
    );
}

// ── 4d.1 / 4d.2 / 4e.1 defenses silence the compile-time warning too ────────

#[test]
fn defended_nullable_rescue_quiet() {
    let source = "\
fn main() {
  z = 5;
  v = [10, 20, 30];
  i = 1;
  a = (10 / z) ?? 0;
  b = v[i] ?? 0;
  print(\"a={a} b={b}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("def_nullable", source);
    assert!(
        !diag.contains("warning: integer division"),
        "?? rescue must silence div warning; got stdout={diag:?}"
    );
    assert!(
        !diag.contains("warning: `v[i]`"),
        "?? rescue must silence vec warning; got stdout={diag:?}"
    );
}

#[test]
fn defended_bare_null_check_quiet() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  i = 1;
  x = v[i];
  if x != null { print(\"got\\n\"); }
}
";
    let (_stdout, diag, _code) = run_with_warnings("def_null_check", source);
    assert!(
        !diag.contains("warning: `v[i]`"),
        "if x != null must silence vec warning; got stdout={diag:?}"
    );
}

#[test]
fn defended_format_string_quiet() {
    let source = "\
fn main() {
  z = 5;
  v = [10, 20, 30];
  i = 1;
  print(\"div={10 / z} vec={v[i]}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("def_fmt", source);
    assert!(
        !diag.contains("warning: integer division"),
        "format-string div must silence warning (4e.1); got stdout={diag:?}"
    );
    assert!(
        !diag.contains("warning: `v[i]`"),
        "format-string vec must silence warning (4e.1); got stdout={diag:?}"
    );
}

// ── Env-var silencing knob ──────────────────────────────────────────────────

#[test]
fn silenced_by_env() {
    let source = "\
fn main() {
  z = 5;
  x = 10 / z;
  print(\"x={x}\\n\");
}
";
    let script_path = std::env::temp_dir().join("loft_w42_silenced.loft");
    fa::write(&script_path, source).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .current_dir(workspace_root())
        .env("LOFT_NO_WARN_RUNTIME", "1")
        .output()
        .expect("failed to invoke loft binary");
    let _ = fa::remove_file(&script_path);
    let diag = String::from_utf8_lossy(&out.stdout);
    assert!(
        !diag.contains("warning: integer division"),
        "LOFT_NO_WARN_RUNTIME=1 must silence the warning; got stdout={diag:?}"
    );
}

// ── Plan-07 phase 4e.3 — distinct null tokens in format-string output ──

/// Format-string interpolation of `1 / z` (z=0) renders `null(/0)`
/// — distinguishes fault-produced null from bare-value null.
#[test]
fn fmt43_div_by_zero_renders_null_div() {
    let source = "\
fn main() {
  z = 0;
  print(\"a={1 / z}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("fmt43_div", source);
    assert_eq!(code, Some(0), "format-string suppression must not halt");
    assert!(
        stdout.contains("a=null(/0)"),
        "expected `null(/0)` suffix; got stdout={stdout:?}"
    );
}

#[test]
fn fmt43_mod_by_zero_renders_null_mod() {
    let source = "\
fn main() {
  z = 0;
  print(\"a={5 % z}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("fmt43_mod", source);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("a=null(%0)"),
        "expected `null(%0)` suffix; got stdout={stdout:?}"
    );
}

#[test]
fn fmt43_vec_oob_renders_null_oob() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  print(\"a={v[999]}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("fmt43_vec", source);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("a=null(oob)"),
        "expected `null(oob)` suffix; got stdout={stdout:?}"
    );
}

#[test]
fn fmt43_genuine_null_renders_bare_null() {
    let source = "\
fn main() {
  z = null as integer?;
  print(\"a={z}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("fmt43_bare", source);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("a=null"),
        "expected bare null; got stdout={stdout:?}"
    );
    assert!(
        !stdout.contains("a=null("),
        "genuine null must not get a fault suffix; got stdout={stdout:?}"
    );
}

// ── Plan-07 phase 4f.12 — stack overflow becomes typed RuntimeError ─────────

/// Infinite recursion produces a typed `StackOverflow` error
/// (rendered with file:line:col and the call chain) instead of
/// an opaque Rust panic.  Production mode logs and continues
/// per C66; dev mode halts and renders.
#[test]
fn f4f_stack_overflow_raises_typed_error() {
    let source = "\
fn recurse(n: integer) -> integer {
  return recurse(n + 1);
}

fn main() {
  x = recurse(0);
  print(\"x={x}\\n\");
}
";
    let (_stdout, stderr, code) = run_with_warnings("f4f_stack", source);
    assert_eq!(code, Some(1), "stack overflow must halt with exit 1");
    // Pretty-renderer + call chain go to stderr per main.rs's
    // `eprint!` / `eprintln!` calls.
    assert!(
        stderr.contains("call stack overflow"),
        "expected typed StackOverflow error; got stderr={stderr:?}"
    );
    assert!(
        stderr.contains("in fn recurse() ← called from"),
        "expected call-chain header pointing at recurse(); got stderr={stderr:?}"
    );
    assert!(
        stderr.contains("more frames"),
        "expected truncation summary; got stderr={stderr:?}"
    );
}

// ── Plan-07 phase 4g — soft-halt + call-chain rendering ─────────────────────

/// `--dev-soft-halt` continues past the first fault and reports
/// every fault site to stderr instead of halting on the first.
#[test]
fn g4g_soft_halt_continues_past_faults() {
    let source = "\
fn main() {
  z = 0;
  bad1 = 1 / z;
  bad2 = 2 / z;
  print(\"done\\n\");
}
";
    let script_path = std::env::temp_dir().join("loft_w42_g4g_soft.loft");
    fa::write(&script_path, source).expect("write");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--dev-soft-halt")
        .arg(&script_path)
        .current_dir(workspace_root())
        .env_remove("LOFT_NO_WARN_RUNTIME")
        .output()
        .expect("invoke loft");
    let _ = fa::remove_file(&script_path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Both faults rendered to stderr.
    assert!(
        stderr.contains("soft-halt: divide by zero")
            && stderr.matches("soft-halt: divide by zero").count() == 2,
        "expected 2 soft-halt fault lines on stderr; got {stderr:?}"
    );
    // Script ran past faults to print "done".
    assert!(
        stdout.contains("done"),
        "soft-halt must continue execution; got stdout={stdout:?}"
    );
    // Exit non-zero because had_fatal was set.
    assert_eq!(
        out.status.code(),
        Some(1),
        "soft-halt with faults must exit 1"
    );
}

/// C80 — an IMPLICIT calculation fault (div0) no longer halts; only an EXPLICIT
/// signal (`panic` / `assert`) halts the run in dev.  This checks the explicit
/// signal still stops at the first one (the post-signal `print` is unreached).
#[test]
fn g4g_explicit_signal_halts_in_dev() {
    let source = "\
fn main() {
  assert(false, \"first\");
  print(\"done\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("g4g_no_soft", source);
    assert_eq!(code, Some(1));
    assert!(
        !stdout.contains("done"),
        "an explicit assert must halt in dev; got stdout={stdout:?}"
    );
}

// NOTE (C80): the former `g4g_call_chain_rendered` tested the call-chain that
// `State::raise()` rendered when an UNDEFENDED div-by-zero dev-halted.  Under
// C80 that fault is recoverable (null-and-continue), so it no longer raises a
// halting error with a chain.  The chain-rendering code still exists (e.g. for
// StackOverflow), but the explicit `panic`/`assert` signals construct their
// error via `RuntimeError::user_panic`/`assertion_failed` with an EMPTY chain
// (n_panic/n_assert take `&mut Stores`, not the interpreter's `State.call_stack`).
// Capturing the chain for those signals is a latent follow-up; the obsolete
// div0-scenario test is removed rather than asserting a behaviour C80 deleted.

// ── Plan-07 phase 4f — float / single div / mod by zero raises ──────────────

/// C80 / E-Report — an undefended float div by zero REPORTS and continues; it is not a
/// dev halt.  The VALUE is IEEE (loft#983): NaN is the float null, so `0.0 / 0.0` is null
/// and `1.0 / 0.0` is `inf`.  This used to expect `null` here while
/// `f4f_float_div_with_nullable_does_not_raise` below expected `inf` for the SAME
/// expression — the two answers one operator gave depending on its destination.
#[test]
fn f4f_float_div_by_zero_reports_and_continues() {
    let source = "\
fn main() {
  z = 0.0;
  bad = 1.0 / z;
  print(\"bad={bad}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("4f_div_float", source);
    assert_eq!(
        code,
        Some(0),
        "report-and-continue exits 0; got code={code:?}"
    );
    assert!(
        stdout.contains("bad=inf"),
        "`1.0 / 0.0` is IEEE `inf` in EVERY destination — bound, returned or inline \
         (loft#983); got stdout={stdout:?}"
    );
}

/// Defended float div by `?? default` does NOT raise — IEEE result
/// (Inf for `1.0 / 0.0`) is returned.
#[test]
fn f4f_float_div_with_nullable_does_not_raise() {
    let source = "\
fn main() {
  z = 0.0;
  good = (1.0 / z) ?? 99.0;
  print(\"good={good}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("4f_div_float_def", source);
    assert_eq!(code, Some(0), "?? defense must not halt");
    // 1.0 / 0.0 = Inf which is NOT null, so `??` doesn't fire.
    assert!(
        stdout.contains("good=inf"),
        "expected IEEE Inf; got stdout={stdout:?}"
    );
}

/// Defended `0.0 / 0.0` (NaN, IS null for floats) `?? default` returns default.
#[test]
fn f4f_float_nan_with_nullable_returns_default() {
    let source = "\
fn main() {
  z = 0.0;
  good = (0.0 / z) ?? 42.0;
  print(\"good={good}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("4f_nan_float_def", source);
    assert_eq!(code, Some(0));
    // 0.0 / 0.0 = NaN which IS null, so `??` fires.
    assert!(
        stdout.contains("good=42"),
        "expected `??` default 42; got stdout={stdout:?}"
    );
}

/// Float mod by zero is null-and-continue like div (C80).
#[test]
fn f4f_float_mod_by_zero_null_continues() {
    let source = "\
fn main() {
  z = 0.0;
  bad = 5.0 % z;
  print(\"bad={bad}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("4f_mod_float", source);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("bad=null"),
        "float mod0 yields null and continues; got stdout={stdout:?}"
    );
}

/// Single-precision div by zero is null-and-continue (C80).
#[test]
fn f4f_single_div_by_zero_reports_and_continues() {
    let source = "\
fn main() {
  z = 0.0f;
  bad = 1.0f / z;
  print(\"bad={bad}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("4f_div_single", source);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("bad=inf"),
        "`single` mirrors `float`: IEEE `inf`, reported and continuing (loft#983); \
         got stdout={stdout:?}"
    );
}

// ── Plan-07 phase 4h — `not null` field-reminder hint ──────────────────

/// @PLN25 DN1/F2 — the `not null` field-reminder hint is RETIRED. A struct field read 10+
/// times with no `??` defense used to trigger a "consider marking it `not null`" hint; under
/// the dense model a plain scalar is NON-null by default (nullability rides `?`), so `not null`
/// is redundant (being retired) and suggesting it is moot — superseded like the div/index fault
/// warnings. This formerly-hinted case must now produce NO such hint.
#[test]
fn hint_4h_high_read_count_hint_retired() {
    let source = "\
struct Player {
  id: integer,
  name: text
}

fn use_player(p: Player) -> integer {
  s = p.id + p.id + p.id + p.id;
  s += p.id + p.id + p.id + p.id;
  s += p.id + p.id + p.id + p.id;
  return s;
}

fn main() {
  p = Player { id: 1, name: \"alice\" };
  print(\"score={use_player(p)}\\n\");
}
";
    let (_stdout, _stderr, code) = run_with_warnings("hint4h_high", source);
    let diag = String::from_utf8_lossy(
        &Command::new(loft_bin())
            .arg("--interpret")
            .arg({
                let p = std::env::temp_dir().join("loft_w42_hint4h_high.loft");
                fa::write(&p, source).expect("write");
                p
            })
            .current_dir(workspace_root())
            .env_remove("LOFT_NO_HINT_NOT_NULL")
            .env_remove("LOFT_NO_WARN_RUNTIME")
            .output()
            .expect("invoke loft")
            .stderr,
    )
    .into_owned();
    assert_eq!(code, Some(0), "must not halt; got code={code:?}");
    assert!(
        !diag.contains("consider marking it `not null`"),
        "the `not null` hint is RETIRED under DN1/F2 — expected NO hint for Player.id; got diag={diag:?}"
    );
}

/// A `not null` field with many reads must NOT trigger the hint.
#[test]
fn hint_4h_already_not_null_quiet() {
    let source = "\
struct Player {
  id: integer not null,
  name: text
}

fn use_player(p: Player) -> integer {
  s = p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id;
  return s;
}

fn main() {
  p = Player { id: 1, name: \"alice\" };
  print(\"score={use_player(p)}\\n\");
}
";
    let p = std::env::temp_dir().join("loft_w42_hint4h_already.loft");
    fa::write(&p, source).expect("write");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&p)
        .current_dir(workspace_root())
        .env_remove("LOFT_NO_HINT_NOT_NULL")
        .env_remove("LOFT_NO_WARN_RUNTIME")
        .output()
        .expect("invoke loft");
    let _ = fa::remove_file(&p);
    let diag = String::from_utf8_lossy(&out.stdout);
    assert!(
        !diag.contains("consider marking it `not null`"),
        "already-not-null field must not get hint; got diag={diag:?}"
    );
}

/// A field read with `?? default` defense (even once) must silence
/// the hint regardless of read count.
#[test]
fn hint_4h_defended_with_nullable_quiet() {
    let source = "\
struct Player {
  id: integer,
  name: text
}

fn use_player(p: Player) -> integer {
  s = (p.id ?? 0);
  s += p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id;
  return s;
}

fn main() {
  p = Player { id: 1, name: \"alice\" };
  print(\"score={use_player(p)}\\n\");
}
";
    let p = std::env::temp_dir().join("loft_w42_hint4h_defended.loft");
    fa::write(&p, source).expect("write");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&p)
        .current_dir(workspace_root())
        .env_remove("LOFT_NO_HINT_NOT_NULL")
        .env_remove("LOFT_NO_WARN_RUNTIME")
        .output()
        .expect("invoke loft");
    let _ = fa::remove_file(&p);
    let diag = String::from_utf8_lossy(&out.stdout);
    assert!(
        !diag.contains("consider marking it `not null`"),
        "?? defended field must not get hint; got diag={diag:?}"
    );
}

/// `LOFT_NO_HINT_NOT_NULL=1` env var silences the hint entirely.
#[test]
fn hint_4h_env_silences() {
    let source = "\
struct Player {
  id: integer,
  name: text
}

fn use_player(p: Player) -> integer {
  s = p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id;
  return s;
}

fn main() {
  p = Player { id: 1, name: \"alice\" };
  print(\"score={use_player(p)}\\n\");
}
";
    let p = std::env::temp_dir().join("loft_w42_hint4h_env.loft");
    fa::write(&p, source).expect("write");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&p)
        .current_dir(workspace_root())
        .env("LOFT_NO_HINT_NOT_NULL", "1")
        .env("LOFT_NO_WARN_RUNTIME", "1")
        .output()
        .expect("invoke loft");
    let _ = fa::remove_file(&p);
    let diag = String::from_utf8_lossy(&out.stdout);
    assert!(
        !diag.contains("consider marking it `not null`"),
        "LOFT_NO_HINT_NOT_NULL=1 must silence hint; got diag={diag:?}"
    );
}

#[test]
fn fmt43_loft_format_bare_null_env_silences_suffix() {
    let source = "\
fn main() {
  z = 0;
  print(\"a={1 / z}\\n\");
}
";
    let script_path = std::env::temp_dir().join("loft_w42_fmt43_env.loft");
    fa::write(&script_path, source).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .current_dir(workspace_root())
        .env("LOFT_FORMAT_BARE_NULL", "1")
        .output()
        .expect("failed to invoke loft binary");
    let _ = fa::remove_file(&script_path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("a=null"),
        "bare null still rendered; got stdout={stdout:?}"
    );
    assert!(
        !stdout.contains("a=null("),
        "LOFT_FORMAT_BARE_NULL=1 must silence the suffix; got stdout={stdout:?}"
    );
}

// ── Index-warning retirement — loop-bounded arithmetic vs mixed index ────────
//
// @PLN25 index flip — `v[i]` now TYPES `τ?`, so the runtime-null warning is RETIRED
// under DN1 for EVERY index shape (a bare loop var, loop-bounded arithmetic `m[i*4+j]`,
// AND a mixed `v[base+k]`). The fit-proof still keeps the bare loop var / const / guard
// non-null (so those never even need discharge); a not-provably-fit index is `τ?` and the
// developer discharges it (`?? d`) — which is the enforcement the warning used to hint at.
#[test]
fn loop_arith_index_no_warn_retired() {
    let source = "\
fn main() {
  m = [for k in 0..16 { k * 1.0 }];
  sum = 0.0;
  for i in 0..4 {
    for j in 0..4 {
      sum += m[i * 4 + j] ?? 0.0;
    }
  }
  print(\"sum={sum}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("skip_loop_arith", source);
    assert!(
        !diag.contains("warning: `v[i]` may produce null"),
        "the v[i] warning is retired under DN1; got stderr={diag:?}"
    );
    // A mixed index `v[base + k]` (base a parameter) is `integer?` and discharged with `?? 0`;
    // it no longer WARNS either — the type + N-Store is the enforcement now.
    let source2 = "\
fn at(v: vector<integer>, base: integer) -> integer {
  total = 0;
  for k in 0..3 { total += v[base + k] ?? 0; }
  total
}
fn main() { print(\"{at([1,2,3,4], 1)}\\n\"); }
";
    let (_o2, diag2, _c2) = run_with_warnings("skip_loop_arith_neg", source2);
    assert!(
        !diag2.contains("warning: `v[i]` may produce null"),
        "the v[i] warning is retired under DN1; got stderr={diag2:?}"
    );
}

// ── `??` after a fault-prone op is a real defense, not "redundant" ──────────
//
// Indexing a `not null` vector, or dividing by a `not null` field, can still
// yield null (out-of-bounds / divide-by-zero).  So `v[i] ?? d` and
// `a / s.field ?? d` must NOT be flagged "Redundant null coalescing" even
// though the vector / field operand carries `not null`.  Without the fix the
// vector-index case and the division case hit contradictory checks (the OOB /
// div warning said "defend me", the redundant check said "the `??` is
// useless").  Surfaced by lib/graphics/src/math.loft + graphics.loft.
#[test]
fn coalesce_after_fault_op_not_redundant() {
    let source = "\
struct M { m: vector<float> not null }
struct S { stride: integer not null }
struct T { steep: float not null }
fn pick(x: const M, i: integer) -> float { x.m[i] ?? 0.0 }
fn cnt(s: const S, n: integer) -> integer { (n / s.stride) ?? 0 }
fn relief(t: const T) -> float { sqrt(max(t.steep, 0.01)) ?? 0.0 }
fn main() {
  a = pick(M { m: [1.0, 2.0] }, 0);
  b = cnt(S { stride: 2 }, 10);
  c = relief(T { steep: 4.0 });
  print(\"a={a} b={b} c={c}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("coalesce_fault_op", source);
    // @PLN102 case E — a domain-fault CALL (`sqrt`, …) nulls a non-null input, so
    // `sqrt(not_null_field) ?? d` is a real defense, exactly like the index/division
    // cases.  The lint gates on the (Optional) result type, not the stale not-null flag.
    assert!(
        !diag.contains("Redundant null coalescing"),
        "`??` after an index / division / sqrt must NOT be flagged redundant; got stderr={diag:?}"
    );
}

// ── a PATTERN's captures must not leak `expr_not_null` into the arm body ──────
//
// `expr_not_null` is a TRANSIENT marker ("the field access just parsed is non-null")
// consumed by the very next operator.  A match PATTERN reads the subject's fields to
// bind its captures, so the LAST capture parsed leaves the marker set with nothing in
// the pattern to consume it, and the arm BODY inherited it — the same leak the
// statement boundary was already reset for.  The first `??` in the body then reported
// that stale NAME: below, `r` is bound only by the second alternative and is not
// mentioned in the body at all, yet the warning named it.
//
// It is not a cosmetic misattribution.  On the sibling shape `(p ?? 0) + (r ?? 0)` the
// same message told the author to delete the `r ?? 0` that is doing the work — `r` is
// null whenever the FIRST alternative matched (types.md `P-Alt-Diff`), so taking the
// advice turns the arm into `p + r`, which evaluates to null.
#[test]
fn pattern_captures_do_not_leak_not_null_into_arm_body() {
    let source = "\
enum E { A { p: integer }, B { q: integer }, C { r: integer } }
fn f(v: vector<E>) -> integer {
  match v { [(A { p } B { q } |A { p } C { r })] => (p ?? 0) + (q ?? 0), _ => -1, }
}
fn main() { print(\"x={f([E::A { p: 1 }, E::B { q: 2 }])}\\n\"); }
";
    let (stdout, diag, _code) = run_with_warnings("pattern_capture_not_null_leak", source);
    assert!(
        !diag.contains("Redundant null coalescing"),
        "a pattern capture must not leak its not-null marker into the arm body; \
         got stderr={diag:?}"
    );
    assert!(
        stdout.contains("x=3"),
        "value must be unchanged; got stdout={stdout:?}"
    );
}

// The other direction: resetting at the `=>` must not DISABLE the lint inside an arm.
// A genuinely redundant coalesce in the body re-arms the marker from its own field
// read, so it still warns — and names the field it is actually about.
#[test]
fn redundant_coalesce_inside_arm_body_still_warns() {
    let source = "\
struct S { x: u8 }
enum E { A { p: integer }, C { r: integer } }
fn f(v: vector<E>, s: S) -> integer {
  match v { [(A { p } |C { r })] => (s.x ?? 0) + (p ?? 0) + (r ?? 0), _ => -1, }
}
fn main() { print(\"x={f([E::A { p: 1 }], S { x: 5 })}\\n\"); }
";
    let (stdout, diag, _code) = run_with_warnings("redundant_coalesce_in_arm", source);
    assert!(
        diag.contains("Redundant null coalescing") && diag.contains("'x'"),
        "a genuinely redundant `s.x ?? 0` inside an arm body must still warn, naming 'x'; \
         got stderr={diag:?}"
    );
    assert!(
        stdout.contains("x=6"),
        "value must be unchanged; got stdout={stdout:?}"
    );
}

// ── @PLN102 case E sibling: `== null` after a fault op is not always-constant ──
// The redundant-null-CHECK lint shares the `expr_not_null` root: `sqrt(field) == null`
// is a genuine check (sqrt of a non-null CAN be null), so it must not warn "always
// false".  A genuine non-null field check still warns (asserted elsewhere: p285).
#[test]
fn null_check_after_fault_op_not_redundant() {
    let source = "\
struct T { steep: float not null }
fn is_bad(t: const T) -> boolean { sqrt(t.steep) == null }
fn main() { print(\"{is_bad(T { steep: 4.0 })}\\n\"); }
";
    let (_stdout, diag, _code) = run_with_warnings("nullcheck_fault_op", source);
    assert!(
        !diag.contains("Redundant null check"),
        "`== null` after a sqrt must NOT be flagged always-constant; got stderr={diag:?}"
    );
}

// ── @P368b — a non-zero NAMED-CONSTANT divisor must not warn ─────────────────
// `const K = 2.0; x / K` used to fire the divide-by-zero warning (the skip-
// pattern only saw literal divisors).  Const propagation now folds the named
// constant to its literal value, so a non-zero named const divisor is
// skip-pattern-1 — no warning.  A genuine variable divisor still warns.
#[test]
fn skip_named_const_nonzero_divisor() {
    let source = "\
const K = 2.0;
const N = 4;
fn main() {
  x = 10.0; y = 20;
  a = x / K;
  b = y / N;
  print(\"a={a} b={b}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("skip_named_const_div", source);
    assert!(
        !diag.contains("division may produce null"),
        "@P368b: non-zero named-const divisor must NOT warn; got stderr={diag:?}"
    );
    // @PLN25 DN3 — a variable divisor no longer warns either: `/` now TYPES `integer?`,
    // so the null lives in the type and `(N-Store)` is the enforcement; the runtime-null
    // warning is retired for Div/Rem under DN1. (The const case above is subsumed but kept
    // as documentation of the fit-proof path.)
    let source2 = "\
fn main() {
  x = 10.0;
  c = ticks();
  d = x / c;
  print(\"d={d}\\n\");
}
";
    let (_o2, diag2, _c2) = run_with_warnings("named_const_div_neg", source2);
    assert!(
        !diag2.contains("division may produce null"),
        "the div warning is retired under DN1 (the type carries the null); got stderr={diag2:?}"
    );
}

// ── Skip pattern 5 — guarded STRUCT FIELD index ─────────────────────────────
// `if i < len(self.v) { self.v[i] }` must be proven safe exactly like the
// bare-local form.  Since a struct vector-field read COPIES (@PLN85 #415),
// value-correct code indexes the field directly rather than a captured alias.

#[test]
fn skip_field_guarded_vec_index() {
    let source = "\
struct S { v: vector<text> not null }
fn rd(self: S, i: integer) -> text {
  if i < len(self.v) { self.v[i] } else { \"\" }
}
fn main() {
  s = S { v: [\"a\", \"b\"] };
  print(\"{rd(s, 0)}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("skip_field_guard", source);
    assert!(
        !diag.contains("may produce null on out-of-bounds"),
        "a `if i < len(self.v) {{ self.v[i] }}` guard must NOT warn; got stderr={diag:?}"
    );
}

#[test]
fn skip_field_guarded_captured_len() {
    let source = "\
struct S { v: vector<text> not null }
fn rd(self: S, i: integer) -> text {
  n = len(self.v);
  if i < n { self.v[i] } else { \"\" }
}
fn main() {
  s = S { v: [\"a\"] };
  print(\"{rd(s, 0)}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("skip_field_caplen", source);
    assert!(
        !diag.contains("may produce null on out-of-bounds"),
        "captured `n = len(self.v); if i < n {{ self.v[i] }}` must NOT warn; got stderr={diag:?}"
    );
}

#[test]
fn field_vec_index_no_warn_retired() {
    // @PLN25 — an UNguarded `self.v[i]` types `text?`; the runtime-null warning is retired
    // (the type + N-Store enforce). An inferred local absorbs it, and the `x != null` check
    // narrows it — the DN1 idiom the warning used to hint at.
    let source = "\
struct S { v: vector<text> not null }
fn rd(self: S, i: integer) -> boolean {
  x = self.v[i];
  x != null
}
fn main() {
  s = S { v: [\"a\"] };
  print(\"{rd(s, 0)}\\n\");
}
";
    let (stdout, diag, _code) = run_with_warnings("undef_field", source);
    assert!(
        !diag.contains("may produce null on out-of-bounds"),
        "the self.v[i] warning is retired under DN1; got stderr={diag:?}"
    );
    assert!(
        stdout.contains("true"),
        "rd(s,0) reads a present element; got {stdout:?}"
    );
}

#[test]
fn wrong_field_guard_still_rejects() {
    // @PLN25 index flip — a guard proves `i < len(self.v)` but the index is on a DIFFERENT
    // field `self.w` (different VecKey), so `self.w[i]` stays `text?`. The runtime-null
    // warning is retired; the TYPE now enforces it — returning the `text?` into the non-null
    // `-> text` produces an `(N-Store)` diagnostic: a hard REJECT by default, relaxed to a
    // WARNING under `LOFT_NULLFLOW` (@PLN102 N-Warn — `text` reserves its null out-of-band).
    // Either way the diagnostic fires; assert on the wording shared by both. (A correct
    // `i < len(self.w)` guard, or a `?? ""` discharge, is clean.)
    let source = "\
struct S { v: vector<text> not null, w: vector<text> not null }
fn rd(self: S, i: integer) -> text {
  if i < len(self.v) { self.w[i] } else { \"\" }
}
fn main() {
  s = S { v: [\"a\"], w: [\"b\"] };
  print(\"{rd(s, 0)}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("wrong_field_guard", source);
    assert!(
        diag.contains("stored into the return value of the non-null type"),
        "a guard on a DIFFERENT field does not prove fit → the `text?` produces an N-Store \
         diagnostic (a hard reject by default; a warning under LOFT_NULLFLOW); got stderr={diag:?}"
    );
}

/// @PLN102 (N-Store) — the return-value N-Store diagnostic must name the OFFENDING
/// function, not the FOLLOWING one.  Regression for the position misreport fixed by
/// doc/claude/plans/102-stability-contract/nstore-position-fix.md: `n_store_violation`
/// used the lexer cursor, which at BLOCK FINALIZATION sits on the next `fn` (the implicit
/// tail is checked after the block is fully parsed).  The nullable tail (`self.w[i]`, a
/// guard on a DIFFERENT field) lives in `read_off` at line 2; the warning must point at
/// line 2, never line 5 (`other_fn`).
#[test]
fn nstore_return_position_names_offending_fn() {
    let source = "\
struct S { v: vector<text>, w: vector<text> }
fn read_off(self: S, i: integer) -> text {
  if i < len(self.v) { self.w[i] } else { \"\" }
}
fn other_fn(x: integer) -> integer {
  x + 1
}
fn main() {
  s = S { v: [\"a\"], w: [\"b\"] };
  print(\"{read_off(s, 0)} {other_fn(2)}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("nstore_return_pos", source);
    assert!(
        diag.contains("stored into the return value"),
        "the nullable tail produces a return-value N-Store; got stderr={diag:?}"
    );
    assert!(
        diag.contains(".loft:2:") && !diag.contains(".loft:5:"),
        "the diagnostic must anchor to the OFFENDING fn `read_off` (line 2), not the \
         FOLLOWING `other_fn` (line 5); got stderr={diag:?}"
    );
}

/// @PLN102 (N-Store) — the SAME position fix for a TAIL call's argument (also checked at
/// block finalization).  The nullable arg `v[i]` is in `pass_off` at line 3; the argument
/// N-Store must point at line 3, not line 5 (`other_fn`).
#[test]
fn nstore_tail_arg_position_names_offending_fn() {
    let source = "\
fn sink(n: integer) -> integer { n }
fn pass_off(v: vector<integer>, i: integer) -> integer {
  sink(v[i])
}
fn other_fn(x: integer) -> integer {
  x + 1
}
fn main() {
  a: vector<integer> = [7];
  print(\"{pass_off(a, 0)} {other_fn(2)}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("nstore_tail_arg_pos", source);
    assert!(
        diag.contains("stored into parameter"),
        "the nullable tail argument produces a call-arg (parameter) N-Store; got stderr={diag:?}"
    );
    assert!(
        diag.contains(".loft:3:") && !diag.contains(".loft:5:"),
        "the diagnostic must anchor to the call in `pass_off` (line 3), not the FOLLOWING \
         `other_fn` (line 5); got stderr={diag:?}"
    );
}

/// A warning pass that seeks the lexer to its diagnostic site must not move the position
/// every LATER diagnostic in the file is derived from.  `check_ref_mutations` (the
/// `needless-const-parameter` / needless-`&` pass) seeks with `Lexer::to` and runs OUTSIDE
/// the save/restore that wraps the other per-function warning passes, so every position
/// after such a function was short by the distance of that seek: in
/// `685-mutated-scalar-param-capture.loft` all nineteen `assert`s reported seven lines
/// early, and a failure printed ANOTHER assert's source under the caret while carrying
/// this one's message.
///
/// It is `assert` that makes the damage reach a user rather than only a caret: the line
/// the compiler injects into the call is read from the same cursor.  So the cell asserts
/// both ends — the advice lands on the `const` it is about, and the failure that comes
/// AFTER it names its own line.
#[test]
fn a_seek_to_a_warning_site_does_not_shift_later_positions() {
    let source = "\
fn scaled(n: const integer) -> integer {
  n * 2
}
fn main() {
  assert(scaled(2) == 5, \"MARK\");
}
";
    let (_stdout, diag, code) = run_with_warnings("const_param_seek", source);
    assert!(
        diag.contains("needless-const-parameter") && diag.contains(".loft:1:14"),
        "the advice points at the `const` token on line 1; got stderr={diag:?}"
    );
    assert!(
        diag.contains("assertion failed: MARK"),
        "the assert must fail so its position is reported; got stderr={diag:?}"
    );
    assert!(
        diag.contains(".loft:5:") && !diag.contains(".loft:2:"),
        "the failing assert is on line 5; a dangling reporting seek made it report line 2 \
         (the body of `scaled`); got stderr={diag:?}"
    );
    assert_eq!(code, Some(1), "a failed assert exits 1");
}

// ── @PLN46 W2 — `#null_safe` param annotation (skip pattern 6) ───────────────

/// A fault-prone expression passed DIRECTLY to a `#null_safe` function is not
/// flagged — the callee contracts to handle the possible null.
#[test]
fn skip_null_safe_arg() {
    let source = "\
fn handles_null(c: character) -> boolean { c == 'a' }
#null_safe
fn scan(line: text, j: integer) -> boolean { handles_null(line[j]) }
fn main() -> integer { 0 }
";
    let (_stdout, diag, _code) = run_with_warnings("w2_null_safe", source);
    assert!(
        !diag.contains("may produce null"),
        "an index passed to a #null_safe param must NOT warn; got stderr={diag:?}"
    );
}

/// The SAME call shape on an UNannotated function still warns — the annotation is
/// the signal, never "the helper happens to avoid the unsafe op".
#[test]
fn null_safe_only_skips_annotated() {
    let source = "\
fn no_annotation(c: character) -> boolean { c == 'a' }
fn scan(line: text, j: integer) -> boolean { no_annotation(line[j]) }
fn main() -> integer { 0 }
";
    let (_stdout, diag, _code) = run_with_warnings("w2_unannotated", source);
    // @PLN25 — the fault warning is RETIRED under DN1 (the type + N-Store enforce), so
    // `#null_safe` warning-suppression is now moot: an undischarged index is `τ?` regardless
    // of the annotation. Neither annotated nor unannotated warns.
    assert!(
        !diag.contains("may produce null"),
        "the index warning is retired under DN1 (#null_safe superseded); got stderr={diag:?}"
    );
}

/// PRECISION: the suppression is direct-argument only — a fault op buried inside a
/// NON-null-safe nested call still warns, even when the OUTER call is `#null_safe`
/// (the inner callee gets the raw null, so the prompt must stand).
#[test]
fn null_safe_does_not_leak_through_nested_call() {
    let source = "\
fn outer(b: boolean) -> boolean { b }
#null_safe
fn raw(c: character) -> boolean { c == 'a' }
fn scan(line: text, j: integer) -> boolean { outer(raw(line[j])) }
fn main() -> integer { 0 }
";
    let (_stdout, diag, _code) = run_with_warnings("w2_nested", source);
    // @PLN25 — fault warning retired under DN1 (#null_safe superseded); no warning fires.
    assert!(
        !diag.contains("may produce null"),
        "the index warning is retired under DN1; got stderr={diag:?}"
    );
}

/// @PLN46 W2 — the STDLIB character predicates are annotated `#null_safe` (they
/// are native `char::is_X()`, which never fault — verified on both backends), so
/// the common `s[i].is_numeric()` shape no longer false-warns, while a raw `s[i]`
/// still does.
#[test]
fn stdlib_char_predicate_is_null_safe() {
    let source = "\
fn scan(line: text, j: integer) -> boolean { line[j].is_numeric() }
fn raw(line: text, j: integer) -> character { line[j] }
fn main() -> integer { 0 }
";
    let (_stdout, diag, _code) = run_with_warnings("w2_stdlib_isnum", source);
    // @PLN25 — the fault warning is retired under DN1 (#null_safe superseded): neither the
    // raw `line[j]` nor the `is_numeric(s[i])` receiver warns; both are `character?` and the
    // type + N-Store is the enforcement.
    assert_eq!(
        diag.matches("may produce null").count(),
        0,
        "the index warning is retired under DN1; got stderr={diag:?}"
    );
}

// ── @PLN46 W3 — entry-guard auto-inference of #null_safe ─────────────────────

/// A function whose every parameter is entry-guarded by `if p == null { return }`
/// is AUTO-inferred `#null_safe`, so a fault-prone arg passed to it is not flagged
/// — no annotation needed.
#[test]
fn w3_entry_guard_infers_null_safe() {
    let source = "\
fn g(c: character) -> boolean { if c == null { return false } c == 'a' }
fn use_g(line: text, j: integer) -> boolean { g(line[j]) }
fn main() -> integer { 0 }
";
    let (_stdout, diag, _code) = run_with_warnings("w3_guard", source);
    assert!(
        !diag.contains("may produce null"),
        "an arg to an entry-guarded (null_safe-inferred) fn must not warn; got stderr={diag:?}"
    );
}

/// SOUNDNESS: a function where only SOME parameters are guarded is NOT inferred
/// null_safe — an arg in the unguarded slot still warns (no over-inference).
#[test]
fn w3_partial_guard_stays_warning() {
    let source = "\
fn h(a: character, b: character) -> boolean { if a == null { return false } a == b }
fn use_h(line: text, j: integer) -> boolean { h('x', line[j]) }
fn main() -> integer { 0 }
";
    let (_stdout, diag, _code) = run_with_warnings("w3_partial", source);
    // @PLN25 — fault warning retired under DN1 (#null_safe inference superseded); `line[j]`
    // is `character?` and no longer warns regardless of the callee's guard shape.
    assert!(
        !diag.contains("may produce null"),
        "the index warning is retired under DN1; got stderr={diag:?}"
    );
}

// ── @PLN87 P3 (W4) — redundant `&` on a heap struct param ────────────────────

/// A `&Obj` param that is only field-mutated (never reassigned) gains nothing
/// from the `&` — field mutation propagates regardless.  W4 flags it.
#[test]
fn w4_redundant_amp_warns() {
    let source = "\
struct Obj { x: integer }
fn f(o: &Obj) { o.x = 1; }
fn main() { a = Obj { x: 0 }; f(&a); print(\"{a.x}\\n\"); }
";
    let (_stdout, diag, _code) = run_with_w4("w4_redundant", source);
    assert!(
        diag.contains("only slows it down"),
        "redundant & must warn (W4); got stderr={diag:?}"
    );
    // @PLN131 — "unless you REASSIGN" is the fix's CONDITION now, not message prose. The
    // assertion follows it there rather than being dropped: what it pins is that the one
    // legitimate use of `&` is still named.
    let (_o2, explained, _c2) = run_with_w4_env("w4_redundant_explain", source, true);
    assert!(
        explained.contains("REASSIGN"),
        "the fix must still name the one thing `&` is for; got stderr={explained:?}"
    );
}

/// A `&Obj` param that IS reassigned writes back to the caller — the `&` is
/// load-bearing, so W4 must NOT fire.
#[test]
fn w4_writeback_amp_no_warn() {
    let source = "\
struct Obj { x: integer }
fn f(o: &Obj) { o = Obj { x: 9 }; }
fn main() { a = Obj { x: 0 }; f(&a); print(\"{a.x}\\n\"); }
";
    let (_stdout, diag, _code) = run_with_w4("w4_writeback", source);
    assert!(
        !diag.contains("only slows it down"),
        "a reassigned & is load-bearing — W4 must NOT fire; got stderr={diag:?}"
    );
}

/// A scalar `&integer` is always load-bearing (scalars are by-value) — never W4.
#[test]
fn w4_scalar_amp_no_warn() {
    let source = "\
fn f(n: &integer) { n = 5; }
fn main() { x = 0; f(&x); print(\"{x}\\n\"); }
";
    let (_stdout, diag, _code) = run_with_w4("w4_scalar", source);
    assert!(
        !diag.contains("only slows it down"),
        "scalar & is always needed — W4 must NOT fire; got stderr={diag:?}"
    );
}

// ── loft#1286 — a FORWARDED `&` is not redundant ────────────────────────────
//
// `slow-reference-parameter` asks whether the whole binding is ever reassigned, and a
// FORWARDER never reassigns — its callee does.  So the one shape where the `&` is carrying
// someone else's write-back was exactly the shape a body-local walk read as redundant, and
// taking the advice there SILENTLY LOSES the write-back: the lint fired only on the correct
// spelling and said nothing about the broken one.
//
// `callee_param_reassigns` is the interprocedural half.  It asks about REASSIGNMENT rather
// than about writes on purpose — its sibling `callee_param_writes` also answers yes to a
// FIELD write, which is precisely the case this advice exists to flag, so using it would have
// silenced the lint everywhere instead of at the forwarders.
//
// Falsified by counting: on released `loft 2026.8.0` the program below draws the advice on
// FIVE of its five `&` parameters; here it draws TWO, the field-writing pair.  The count is
// the assertion for that reason — a bare `contains` would have passed on both builds.

const FWD_1286: &str = "\
struct B { items: vector<integer> }
fn reassign(b: &B) { b = B { items: [9] }; }
fn fieldonly(b: &B) { b.items = [7]; }
fn fwd1(b: &B) { reassign(b); }
fn fwd2(b: &B) { fwd1(b); }
fn fwd3(b: &B) { fwd2(b); }
fn fwd_field(b: &B) { fieldonly(b); }
fn main() {
  a = B { items: [0] };  fwd3(a);       print(\"fwd3={a.items}\\n\");
  b = B { items: [0] };  fwd1(b);       print(\"fwd1={b.items}\\n\");
  c = B { items: [0] };  fwd_field(c);  print(\"fwdfield={c.items}\\n\");
  d = B { items: [0] };  reassign(d);   print(\"direct={d.items}\\n\");
}
";

#[test]
fn forwarded_ref_parameter_is_not_advised_away_1286() {
    let (stdout, diag, _code) = run_with_warnings("fwd_ref_1286", FWD_1286);
    // The advice must name ONLY the two field-writing forms.  Counting is what makes this a
    // real assertion: a bare `contains` would pass while the forwarders were still flagged.
    let hits = diag.matches("only slows it down").count();
    assert_eq!(
        hits, 2,
        "expected the advice on `fieldonly` and `fwd_field` and on nothing else; \
         got {hits} occurrences in {diag:?}"
    );
    for quiet in ["fwd1", "fwd2", "fwd3", "reassign"] {
        assert!(
            !diag.contains(&format!("parameter `{quiet}`")),
            "`{quiet}` forwards or performs a reassignment, so its & is load-bearing; \
             got {diag:?}"
        );
    }
    // And the write-back the `&` carries actually arrives — through three levels of
    // forwarding, which is what says the interprocedural walk follows the chain rather than
    // looking one call deep.
    for expect in ["fwd3=[9]", "fwd1=[9]", "fwdfield=[7]", "direct=[9]"] {
        assert!(
            stdout.contains(expect),
            "expected {expect:?} in the output; got {stdout:?}"
        );
    }
}

#[test]
fn a_field_only_ref_parameter_is_still_advised_1286() {
    // The control that keeps the fix honest: widening the predicate must not silence the
    // case the advice exists for.  A `&` whose body only writes a FIELD is redundant, and
    // dropping it keeps the same answer.
    let source = "\
struct S { a: integer }
fn f(s: &S) { s.a = 1; }
fn main() { v = S { a: 0 }; f(v); print(\"a={v.a}\\n\"); }
";
    let (stdout, diag, _code) = run_with_warnings("field_only_1286", source);
    assert!(
        diag.contains("only slows it down"),
        "a field-only & is still redundant and must still be advised; got {diag:?}"
    );
    assert!(
        stdout.contains("a=1"),
        "the field write lands; got {stdout:?}"
    );
}

// ── loft#1284 — (N-Store) reaches a TUPLE ELEMENT destination ───────────────
//
// `(N-Store)` covers the direct store, the field, the call-argument site and the branch join
// (`D-Null-Join`), and a tuple ELEMENT reached none of them: the tuple-element assign branch
// returns before the general assign path that asks.  So `s.i = null` on a non-null FIELD
// warned while `c.1 = null` on a non-null ELEMENT said nothing, for the same store into the
// same kind of slot.
//
// Both halves are asserted here rather than in a `.loft` guard, because half the claim is an
// ABSENCE — the nullable spellings must stay QUIET — and a script guard can pin a
// diagnostic's presence but not its absence.
//
// Falsified by counting: released `loft 2026.8.0` emits ZERO warnings for the program below —
// not three of five, none — and here it emits three, on exactly the non-null destinations.

const NSTORE_TUPLE_1284: &str = "\
fn maybe(k: integer) -> integer? { if k > 0 { k } else { null } }
fn main() {
  a = (1, 5);                       a.1 = null;      print(\"1={a.1}\\n\");
  b: (integer, integer?) = (1, 5);  b.1 = null;      print(\"2={b.1}\\n\");
  c = (1, 5);                       c.1 = maybe(0);  print(\"3={c.1}\\n\");
  d: (integer, integer?) = (1, 5);  d.1 = maybe(0);  print(\"4={d.1}\\n\");
  e = (1, \"s\");                     e.1 = null;      print(\"5={e.1}\\n\");
}
";

#[test]
fn n_store_warns_on_a_non_null_tuple_element_1284() {
    let (stdout, diag, _code) = run_with_warnings("nstore_tuple_1284", NSTORE_TUPLE_1284);
    // THREE non-null destinations and two nullable ones.  The count is the assertion: a bare
    // `contains` would pass while the nullable rows were also being flagged.
    let hits = diag.matches("the tuple element").count();
    assert_eq!(
        hits, 3,
        "expected (N-Store) on the three NON-NULL element destinations and on neither \
         nullable one; got {hits} in {diag:?}"
    );
    // The bare-`null` form and the `τ?`-value form are different branches of the check, so
    // each is named rather than trusting the count alone.
    assert!(
        diag.contains(
            "`null` is stored into the tuple element of the non-null scalar type `integer`"
        ),
        "the bare-null branch did not fire; got {diag:?}"
    );
    assert!(
        diag.contains("a nullable `integer?` is stored into the tuple element"),
        "the nullable-value branch did not fire; got {diag:?}"
    );
    assert!(
        diag.contains("non-null scalar type `text`"),
        "the text element was not covered; got {diag:?}"
    );
    // The store PROCEEDS in every row — the warning is a nudge, not a refusal, and the slot
    // holds the sentinel (loft#1282).
    for expect in ["1=null", "2=null", "3=null", "4=null", "5=null"] {
        assert!(
            stdout.contains(expect),
            "expected {expect:?} — the store proceeds and the slot reads back null; \
             got {stdout:?}"
        );
    }
}

#[test]
fn n_store_keeps_its_hard_error_for_a_narrow_tuple_element_1284() {
    // The warn/error split is `(N-Store)`'s, not the destination's: a NARROW width spends its
    // whole range on real values, so a null there would silently corrupt and stays a hard
    // ERROR — for a tuple element exactly as for a field.
    let source = "\
fn main() {
  n: (integer, u8) = (1, 5);
  n.1 = null;
  print(\"v={n.1}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("nstore_narrow_1284", source);
    assert!(
        diag.contains("cannot be stored into the tuple element"),
        "a narrow element must keep the hard error, not soften to a warning; got {diag:?}"
    );
}

// ── The "never null" lints ask the slot's TYPE (formal/types.md D-types-9) ──
// Four lints claim a field read can never be null: `!s.f`, `s.f == null`, `s.f ?? d`, `s.f?`.
// That claim is true of a declared narrow range (C127), a dense record and a collection, and
// false of every in-band kind, whose non-null slot holds the null `(N-Store)` lets into it.
// Each cell scores the lint AND the value it answers, because a lint that went quiet by
// changing the answer would pass a diagnostic-only check.

/// `(kind, field type, the stored value, the `??` default, the four expected answers in
/// order `!`, `== null`, `??`, `?`, whether the lints may speak)`.
type NeverNullCell = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    [&'static str; 4],
    bool,
);

const NEVER_NULL_KINDS: &[NeverNullCell] = &[
    (
        "integer",
        "integer",
        "null",
        "7",
        ["true", "true", "7", "0"],
        false,
    ),
    (
        "float",
        "float",
        "null",
        "7.5",
        ["true", "true", "7.5", "0"],
        false,
    ),
    (
        "text",
        "text",
        "null",
        "\"d\"",
        ["true", "true", "d", ""],
        false,
    ),
    (
        "enum",
        "Col",
        "null",
        "Col.Green",
        ["true", "true", "Green", "Red"],
        false,
    ),
    ("u8", "u8", "5", "7", ["false", "false", "5", "5"], true),
    (
        "record",
        "P",
        "null",
        "P { x: 7 }",
        ["false", "false", "{x:0}", "{x:0}"],
        true,
    ),
    (
        "vector",
        "vector<integer>",
        "null",
        "[7]",
        ["false", "false", "[]", "[]"],
        true,
    ),
];

const NEVER_NULL_FORMS: [(&str, &str, &str); 4] = [
    ("negation", "!s.f", "redundant-null-negation"),
    ("check", "s.f == null", "redundant-null-check"),
    ("coalesce", "s.f ?? DEFAULT", "redundant-coalesce"),
    ("fallback", "s.f?", "redundant-default-fallback"),
];

#[test]
fn never_null_lints_speak_only_where_the_slot_has_no_null() {
    let mut wrong = Vec::new();
    for (kind, tp, stored, default, answers, speaks) in NEVER_NULL_KINDS {
        for (i, (form, expr, code)) in NEVER_NULL_FORMS.iter().enumerate() {
            let expr = expr.replace("DEFAULT", default);
            let source = format!(
                "enum Col {{ Red, Green }}\n\
                 struct P {{ x: integer }}\n\
                 struct S {{ f: {tp} }}\n\
                 fn main() {{ s = S {{ f: {stored} }}; print(\"<{{{expr}}}>\"); }}\n"
            );
            let (stdout, diag, _) =
                run_with_warnings(&format!("never_null_{kind}_{form}"), &source);
            let want = format!("<{}>", answers[i]);
            if !stdout.contains(&want) {
                wrong.push(format!("{kind} {form}: printed {stdout:?}, want {want}"));
            }
            let spoke = diag.contains(&format!("warning[{code}]"));
            if spoke != *speaks {
                wrong.push(format!(
                    "{kind} {form}: {code} spoke={spoke}, want {speaks}"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "never-null lint cells:\n{}",
        wrong.join("\n")
    );
}

/// The cell nothing else reports: an overflow reaches a plain `integer` field with no
/// `(N-Store)` warning at all (C85 types `*` non-null), so the lint was the ONLY diagnostic
/// on the line, and it advised deleting the one test that sees the null.
#[test]
fn never_null_lints_are_quiet_on_an_overflowed_integer_field() {
    let source = "struct S { i: integer }\n\
                  fn big() -> integer { 4611686018427387904 }\n\
                  fn main() { s = S { i: 1 }; s.i = big() * 4; \
                  print(\"<{!s.i} {s.i == null} {s.i ?? 7}>\"); }\n";
    let (stdout, diag, _) = run_with_warnings("never_null_overflow", source);
    assert!(
        stdout.contains("<true true 7>"),
        "the overflow reads null; got {stdout:?}"
    );
    assert!(
        !diag.contains("warning[redundant-"),
        "no lint may call the overflowed field never null; got stderr={diag:?}"
    );
}
