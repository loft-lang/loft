// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-07 phase 4 production-mode tests.
//!
//! Asserts the production-mode log-and-continue contract from
//! [`DESIGN_DECISIONS.md § C66`](../doc/claude/DESIGN_DECISIONS.md#c66--production-loft-programs-never-abort-on-user-attributable-edge-cases-development-may-halt):
//! when the loft binary is invoked with `--production` and a logger
//! is attached, runtime fault sites (panic / assert / div / mod /
//! vec OOB / text OOB / null deref / narrow cast / stack overflow)
//! MUST log the typed event AND let execution continue with the
//! existing silent sentinel — they MUST NOT halt mid-program.
//!
//! Each cell:
//! - writes a loft script to a tempdir
//! - drops a `log.conf` beside it pointing at a known `log.txt` path
//! - invokes `loft --interpret --production <script>`
//! - asserts post-fault stdout reached (program continued past the
//!   fault site)
//! - asserts the log file contains the expected runtime-event entry
//!   shape (`[<kind_label>] <description>`)
//! - asserts the process exits 1 (had_fatal still triggers exit 1
//!   so the host knows something bad happened — the program
//!   completed without halt, then exits at end)
//!
//! Dev-mode behaviour (halt + render) is covered separately by
//! `tests/runtime_errors.rs`.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Set up a tempdir + script + log.conf for a production-mode run.
/// Returns (script_path, log_path).  The log.conf points at the
/// log_path so we can inspect captured entries afterwards.
fn setup_prod_run(name: &str, source: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "loft_prod_{}_{}_{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let script_path = dir.join(format!("{name}.loft"));
    std::fs::write(&script_path, source).expect("write script");
    let log_path = dir.join("log.txt");
    let conf = format!("[log]\nfile = {}\nlevel = info\n", log_path.display());
    std::fs::write(dir.join("log.conf"), conf).expect("write log.conf");
    (script_path, log_path)
}

/// Run a script under `--interpret --production` and return
/// (stdout, stderr, exit-code, captured-log).
fn run_prod(name: &str, source: &str) -> (String, String, Option<i32>, String) {
    let (script_path, log_path) = setup_prod_run(name, source);
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--production")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(script_path.parent().unwrap());
    (stdout, stderr, out.status.code(), log)
}

/// Production: `panic("msg")` logs Fatal + continues + exits 1
/// (had_fatal).  Pre-panic stdout reaches; post-panic stdout
/// also reaches because the program continues past the panic call.
#[test]
fn prod_user_panic_logs_and_continues() {
    let source = "\
fn main() {
  print(\"before\\n\");
  panic(\"boom\");
  print(\"after\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("user_panic", source);
    assert_eq!(code, Some(1), "had_fatal should still exit 1");
    assert!(
        stdout.contains("before"),
        "pre-panic stdout missing; got {stdout:?}"
    );
    assert!(
        stdout.contains("after"),
        "production must continue past panic; post-panic stdout missing; got {stdout:?}"
    );
    assert!(
        log.contains("FATAL") && log.contains("[user_panic]") && log.contains("boom"),
        "log missing user_panic Fatal entry; got {log:?}"
    );
}

/// Production: failed `assert(false, "msg")` logs Error + continues + exits 1.
#[test]
fn prod_assertion_failed_logs_and_continues() {
    let source = "\
fn main() {
  print(\"before\\n\");
  assert(2 + 2 == 5, \"math is broken\");
  print(\"after\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("assertion_failed", source);
    assert_eq!(code, Some(1), "had_fatal should still exit 1");
    assert!(stdout.contains("before"));
    assert!(
        stdout.contains("after"),
        "production must continue past failed assert; got {stdout:?}"
    );
    assert!(
        log.contains("ERROR")
            && log.contains("[assertion_failed]")
            && log.contains("math is broken"),
        "log missing assertion_failed Error entry; got {log:?}"
    );
}

/// C80 / E-Uncomp: an UNGUARDED integer `/` by zero is a recoverable
/// calculation fault — it logs a Warn (the "no guard" report) + returns the
/// null sentinel + CONTINUES, mode-independently (exit 0, like OOB below).  A
/// `?? null` / null-check guard would emit the silent `*Nullable` op instead.
#[test]
fn prod_divide_by_zero_logs_and_continues() {
    let source = "\
fn main() {
  print(\"before\\n\");
  a = 10;
  b = 0;
  c = a / b;
  print(\"after\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("divide_by_zero", source);
    assert_eq!(code, Some(0), "recoverable div-by-zero should exit 0 (C80)");
    assert!(stdout.contains("before"));
    assert!(
        stdout.contains("after"),
        "must continue past divide-by-zero; got {stdout:?}"
    );
    assert!(
        log.contains("WARN") && log.contains("[divide_by_zero]"),
        "unguarded div-by-zero must report a Warn log; got {log:?}"
    );
}

/// loft#984 — a value outside a DECLARED range reaching the slot at run time takes the
/// slot's default (the lowest value in range, or null where the slot admits it) and reports
/// a Warn, on the same recoverable channel as div-by-zero above: the run continues and
/// exits 0.  The value the program computed is not the value the slot holds, so it is
/// reported; it is not a halt, because one degraded value never stops the rest (C80).
///
/// The one seam such a value reaches at run time is the slot's own arithmetic stepping past
/// its range (`+=`, C85's overflow edge): a plain store of an unranged value — `v = n` —
/// is refused at compile time for a `limit` slot exactly as for `u8` since loft#1593, and
/// a refused program exits 1 before any log line exists.
#[test]
fn prod_range_defaulted_logs_and_continues() {
    let source = "\
fn main() {
  print(\"before\\n\");
  v: integer limit(10, 20) = 12;
  n = 87;
  v += n;
  print(\"v={v}\\n\");
  print(\"after\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("range_defaulted", source);
    assert_eq!(
        code,
        Some(0),
        "a defaulted store is recoverable — exit 0 (C80)"
    );
    assert!(stdout.contains("before"));
    assert!(
        stdout.contains("v=10"),
        "the slot takes the LOWEST value in its range, not zero; got {stdout:?}"
    );
    assert!(
        stdout.contains("after"),
        "must continue past the defaulted store; got {stdout:?}"
    );
    assert!(
        log.contains("WARN") && log.contains("[range_defaulted]"),
        "an out-of-range write must report a Warn log; got {log:?}"
    );
}

/// Production: vector positive-index OOB logs Warn + returns null DbRef
/// sentinel + continues.  The post-fault read of `x.something` would
/// see null too, so just verify the post-fault stdout reaches.
#[test]
fn prod_index_out_of_bounds_vector_logs_and_continues() {
    let source = "\
fn main() {
  print(\"before\\n\");
  v = [10, 20, 30];
  _x = v[5];
  print(\"after\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("index_out_of_bounds_vector", source);
    // @P356: OOB index is a RECOVERABLE fault — logs a Warn and continues
    // without failing the program (exit 0).  Fail-fast is opt-in via
    // `LOFT_DEV_SOFT_HALT` only.
    assert_eq!(code, Some(0), "recoverable OOB should exit 0");
    assert!(stdout.contains("before"));
    assert!(
        stdout.contains("after"),
        "production must continue past OOB; got {stdout:?}"
    );
    assert!(
        log.contains("WARN")
            && log.contains("[index_out_of_bounds]")
            && log.contains("index 5 out of bounds for length 3"),
        "log missing OOB Warn entry; got {log:?}"
    );
}

/// Production: text positive-index OOB logs Warn + returns char(0) +
/// continues.
#[test]
fn prod_index_out_of_bounds_text_logs_and_continues() {
    let source = "\
fn main() {
  print(\"before\\n\");
  s = \"abc\";
  _c = s[100];
  print(\"after\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("index_out_of_bounds_text", source);
    // @P356: recoverable text OOB logs Warn + continues; exit 0.
    assert_eq!(code, Some(0), "recoverable text OOB should exit 0");
    assert!(stdout.contains("before"));
    assert!(
        stdout.contains("after"),
        "production must continue past text OOB; got {stdout:?}"
    );
    assert!(
        log.contains("WARN")
            && log.contains("[index_out_of_bounds]")
            && log.contains("index 100 out of bounds for length 3"),
        "log missing text-OOB Warn entry; got {log:?}"
    );
}

/// Phase 4d.1 — `expr ?? fallback` defense suppresses both the
/// runtime log AND the program-end exit code.  Codegen swaps the
/// outer fault-prone op to its `Nullable` peer (which silently
/// returns the sentinel without calling `s.raise`), then `??`
/// discharges the null with the fallback.  No log noise, exit 0.
/// Mirrors the existing C54.G-hybrid behaviour for `a / b ?? 0`.
#[test]
fn prod_vector_oob_with_nullable_rescue_does_not_log() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  x = v[5] ?? null;
  if x == null { print(\"rescued\\n\"); }
}
";
    let (stdout, _stderr, code, log) = run_prod("vector_oob_nullable_rescue", source);
    assert_eq!(code, Some(0), "?? rescue should exit 0; log={log:?}");
    assert!(
        stdout.contains("rescued"),
        "?? rescue should produce the fallback; got {stdout:?}"
    );
    assert!(
        !log.contains("[index_out_of_bounds]"),
        "?? rescue must NOT emit log entry; got {log:?}"
    );
}

/// Phase 4d.1 — same shape for text indexing.  `s[i] ?? null` on
/// OOB silently produces null.
#[test]
fn prod_text_oob_with_nullable_rescue_does_not_log() {
    let source = "\
fn main() {
  s = \"abc\";
  c = s[100] ?? null;
  if c == null { print(\"rescued\\n\"); }
}
";
    let (stdout, _stderr, code, log) = run_prod("text_oob_nullable_rescue", source);
    assert_eq!(code, Some(0), "?? rescue should exit 0; log={log:?}");
    assert!(stdout.contains("rescued"));
    assert!(
        !log.contains("[index_out_of_bounds]"),
        "?? rescue must NOT emit log entry; got {log:?}"
    );
}

/// Phase 4d.1 — struct-ref vector `vp[i] ?? null` (uses
/// `OpVectorRef`, not `OpGetVector`) — confirms the dispatch covers
/// both index opcodes.
#[test]
fn prod_struct_ref_vector_oob_with_nullable_rescue_does_not_log() {
    let source = "\
struct P { v: integer }
fn main() {
  vp = [P{v:1}, P{v:2}];
  q = vp[5] ?? null;
  if q == null { print(\"rescued\\n\"); }
}
";
    let (stdout, _stderr, code, log) = run_prod("struct_ref_oob_nullable_rescue", source);
    assert_eq!(code, Some(0), "?? rescue should exit 0; log={log:?}");
    assert!(stdout.contains("rescued"));
    assert!(
        !log.contains("[index_out_of_bounds]"),
        "?? rescue must NOT emit log entry; got {log:?}"
    );
}

/// Phase 4d.2 — bare `if x != null { … }` after `x = v[i]`
/// detected as a defensive check; codegen swaps `OpGetVector` to
/// its `Nullable` peer at compile time so neither log nor halt
/// fires.  Mode-independent (works in production AND dev) because
/// the swap happens BEFORE the runtime mode branch.
#[test]
fn prod_vector_oob_with_bare_null_check_does_not_log() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  x = v[5];
  if x != null { print(\"hit\\n\"); }
  else { print(\"rescued\\n\"); }
}
";
    let (stdout, _stderr, code, log) = run_prod("vector_oob_bare_null_check", source);
    assert_eq!(code, Some(0), "bare null-check should exit 0; log={log:?}");
    assert!(
        stdout.contains("rescued"),
        "defensive null-check should fire; got {stdout:?}"
    );
    assert!(
        !log.contains("[index_out_of_bounds]"),
        "defended site must NOT emit log entry; got {log:?}"
    );
}

/// Phase 4d.2 — same shape but using bare truthy `if x { … }`
/// (loft's truthy check is `false` for null DbRef, so this also
/// counts as a defensive check).
#[test]
fn prod_vector_oob_with_bare_truthy_check_does_not_log() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  x = v[5];
  if x { print(\"hit\\n\"); }
  else { print(\"rescued\\n\"); }
}
";
    let (stdout, _stderr, code, log) = run_prod("vector_oob_bare_truthy_check", source);
    assert_eq!(
        code,
        Some(0),
        "bare truthy-check should exit 0; log={log:?}"
    );
    assert!(
        stdout.contains("rescued"),
        "defensive truthy-check should fire; got {stdout:?}"
    );
    assert!(
        !log.contains("[index_out_of_bounds]"),
        "defended site must NOT emit log entry; got {log:?}"
    );
}

/// Production: a clean run produces no runtime-event log entries.
/// Phase 4e.1 — format-string interpolation MUST NEVER halt, log, or
/// warn even when the interpolated expression is undefended.  Format
/// strings are the developer's debugging surface (per C66 + chat
/// 2026-05-11): the `println("{x}")` you reach for to inspect a bug
/// must not itself become the next bug.  Every fault-prone op inside
/// `{...}` is auto-swapped to its Nullable peer at parse time.
#[test]
fn prod_format_string_div_by_zero_does_not_log() {
    let source = "\
fn main() {
  z = 0;
  print(\"a={1 / z}\\n\");
  print(\"b={2 % z}\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("fmt_div_zero", source);
    assert_eq!(code, Some(0), "format-string div should not halt");
    assert!(stdout.contains("a=null"), "got {stdout:?}");
    assert!(stdout.contains("b=null"), "got {stdout:?}");
    assert!(
        !log.contains("[divide_by_zero]"),
        "format-string div MUST NOT log; got {log:?}"
    );
}

/// Phase 4e.1 — format-string vector OOB must not halt or log.
#[test]
fn prod_format_string_vector_oob_does_not_log() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  print(\"v[999]={v[999]}\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("fmt_vec_oob", source);
    assert_eq!(code, Some(0), "format-string OOB should not halt");
    assert!(stdout.contains("v[999]=null"), "got {stdout:?}");
    assert!(
        !log.contains("[index_out_of_bounds]"),
        "format-string OOB MUST NOT log; got {log:?}"
    );
}

/// Guards against the production code emitting spurious log entries
/// for normal program execution.
#[test]
fn prod_clean_run_logs_nothing_runtime() {
    let source = "\
fn main() {
  print(\"hello\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("clean_run", source);
    assert_eq!(code, Some(0), "clean run should exit 0");
    assert!(stdout.contains("hello"));
    // No runtime-event entries should appear.  (The log file may not
    // even exist if nothing ever logged.)
    assert!(
        !log.contains("[user_panic]")
            && !log.contains("[assertion_failed]")
            && !log.contains("[divide_by_zero]")
            && !log.contains("[index_out_of_bounds]"),
        "clean run produced spurious runtime-event log; got {log:?}"
    );
}

// ---------------------------------------------------------------------------
// Defended fault sites — the E-Report half of the `*Nullable` op split.
//
// These are NOT production-mode tests; they share this file only because the
// machinery (script + log.conf + read the captured log) is already here.
// ---------------------------------------------------------------------------

/// Run a script under a plain (non-production) backend with a logger attached,
/// and return the captured log.
fn run_logged(name: &str, source: &str, native: bool) -> String {
    let (script_path, log_path) = setup_prod_run(name, source);
    let conf = script_path.parent().unwrap().join("log.conf");
    let out = Command::new(loft_bin())
        .arg(if native { "--native" } else { "--interpret" })
        .arg("--log-conf")
        .arg(&conf)
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft binary");
    assert!(
        out.status.success(),
        "{name} ({}) did not run cleanly: {}",
        if native { "native" } else { "interp" },
        String::from_utf8_lossy(&out.stderr)
    );
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(script_path.parent().unwrap());
    log
}

/// A fault site the program DEFENDS must not report — for every element type.
///
/// `rewrite_defended_fault_sites` swaps a defended fault-prone op to its
/// Nullable peer, which never calls `s.raise`, so the site stays silent while
/// an undefended one reports.  The two defence spellings reach that swap by
/// different routes: `v[i] ?? fb` through `rewrite_outer_arith_to_nullable`,
/// and `x = v[i]; if x == null {…}` through the defended-fault-site pass.
///
/// Typed-vector indexing emits a type-specific getter WRAPPING the raw
/// `OpGetVector` — `OpGetInt(OpGetVector(…), 0)` and analogues — and it is the
/// inner op that raises, so the swap must descend through the wrapper.  The
/// list of wrappers to descend through existed in two copies; @P356 extended
/// one of them past the integer wrappers and the other kept the four it was
/// born with, so a defended read of a `text` / `float` / `single` / `enum` /
/// `character` / `boolean` vector reported while the identical defence over
/// `vector<integer>` stayed silent.
///
/// ⚠ Asserted on the LOG, not on the value: every cell answers `null` either
/// way, which is why a value-only probe of this exact matrix comes back clean.
/// The element type is the axis — a single-type version of this test passes
/// against the bug whichever type it picks, as long as that type is `integer`.
#[test]
fn a_defended_fault_site_reports_for_no_element_type() {
    let source = "\
fn main() {
  iv: vector<integer> = [7];
  tv: vector<text> = [\"a\"];
  fv: vector<float> = [1.5];
  sv: vector<single> = [1.5f];
  cv: vector<character> = ['x'];
  bv: vector<boolean> = [true];
  a = iv[5] ?? -1;
  print(\"{a}\\n\");
  b = iv[5];
  if b == null { print(\"b\\n\"); }
  c = tv[5];
  if c == null { print(\"c\\n\"); }
  d = fv[5];
  if d == null { print(\"d\\n\"); }
  e = sv[5];
  if e == null { print(\"e\\n\"); }
  f = cv[5];
  if f == null { print(\"f\\n\"); }
  g = bv[5];
  if g == null { print(\"g\\n\"); }
}
";
    for native in [false, true] {
        let log = run_logged("defended_sites", source, native);
        assert!(
            !log.contains("[index_out_of_bounds]"),
            "a defended fault site reported ({}); got {log:?}",
            if native { "native" } else { "interp" }
        );
    }
}

/// The control cell: the same reads, UNDEFENDED, must still report.
///
/// Without this, the test above passes just as well if the swap is applied
/// everywhere — or if reporting is switched off altogether — which would
/// delete the distinction the split exists to draw rather than fix its drift.
#[test]
fn an_undefended_fault_site_still_reports_for_every_element_type() {
    let source = "\
fn main() {
  tv: vector<text> = [\"a\"];
  fv: vector<float> = [1.5];
  cv: vector<character> = ['x'];
  u = tv[5];
  print(\"{u}\\n\");
  w = fv[5];
  print(\"{w}\\n\");
  y = cv[5];
  print(\"{y}\\n\");
}
";
    for native in [false, true] {
        let log = run_logged("undefended_sites", source, native);
        let hits = log.matches("[index_out_of_bounds]").count();
        assert_eq!(
            hits,
            3,
            "an undefended fault site must report ({}); got {log:?}",
            if native { "native" } else { "interp" }
        );
    }
}

/// `(E-Report)` says "a following null-check", not "the very next statement".
///
/// An ordinary statement between the fault site and its guard must not cost the program its
/// silence — the guard still owns the null, because nothing touched the value in between.
#[test]
fn a_null_check_after_an_unrelated_statement_still_owns_the_null() {
    let source = "\
fn main() {
  tv: vector<text> = [\"a\"];
  f = tv[5];
  g = 1;
  if f == null { print(\"{g}\\n\"); }
}
";
    for native in [false, true] {
        let log = run_logged("gap_then_check", source, native);
        assert!(
            !log.contains("[index_out_of_bounds]"),
            "an intervening statement must not defeat the guard ({}); got {log:?}",
            if native { "native" } else { "interp" }
        );
    }
}

/// The spelling with no binding at all — `(E-Report)`'s "a following null-check" reaching
/// the form where the fault site sits INSIDE the test (`D-op-5`).
///
/// `rewrite_defended_fault_sites` keys on a `Set` followed by an `if` that tests the
/// variable, and here there is no `Set` to key on, so the site kept reporting a fault the
/// program had already defended. It needs no adjacency window and no dataflow: the guard is
/// the SAME expression, so the null this site produces is consumed by the comparison and no
/// other reader can observe it.
///
/// Both orders of the comparison and both polarities, because the rewrite picks whichever
/// operand is not the null literal.
#[test]
fn a_fault_site_inside_its_own_null_test_is_guarded() {
    for (tag, guard) in [
        ("eq", "if v[5] == null { print(\"a\\n\"); }"),
        (
            "ne",
            "if v[5] != null { print(\"b\\n\"); } else { print(\"c\\n\"); }",
        ),
        ("rev", "if null == v[5] { print(\"d\\n\"); }"),
        ("or", "if v[5] == null || v[0] == 7 { print(\"e\\n\"); }"),
    ] {
        let source = format!("fn main() {{\n  v: vector<integer> = [7];\n  {guard}\n}}\n");
        for native in [false, true] {
            let log = run_logged(&format!("inline_null_test_{tag}"), &source, native);
            assert!(
                !log.contains("[index_out_of_bounds]"),
                "`{guard}` guards its own fault site ({}); got {log:?}",
                if native { "native" } else { "interp" }
            );
        }
    }
}

/// The control for the cell above, and the one that decides whether it is sound.
///
/// A comparison that is not a NULL test guards nothing — the null flows into it as an
/// ordinary operand and on into the program — so the site still owes its report. This is
/// what stops the rewrite being "any `if` that mentions the site".
#[test]
fn a_fault_site_in_a_non_null_comparison_still_reports() {
    let source = "\
fn main() {
  v: vector<integer> = [7];
  if v[5] > 3 { print(\"big\\n\"); } else { print(\"small\\n\"); }
}
";
    for native in [false, true] {
        let log = run_logged("inline_non_null_cmp", source, native);
        assert!(
            log.contains("[index_out_of_bounds]"),
            "a non-null comparison does not own the null and must still report ({}); got {log:?}",
            if native { "native" } else { "interp" }
        );
    }
}

/// `match x { null => … }` guards its subject, though it names a COPY of it (`D-op-5`).
///
/// The match has no `Value::Match` in the IR — it lowers to a nested block whose first act is
/// `_match_subj_N = x`, so the arms test the temp and the adjacency scan, looking for an `if`
/// testing the variable itself, saw a `Block` and gave up.
#[test]
fn a_match_on_null_guards_its_subject() {
    let source = "\
fn main() {
  v: vector<integer> = [7];
  x = v[5];
  r = match x { null => \"n\", _ => \"v\" };
  print(\"{r}\\n\");
}
";
    for native in [false, true] {
        let log = run_logged("match_null_guard", source, native);
        assert!(
            !log.contains("[index_out_of_bounds]"),
            "a `null` match arm owns the null ({}); got {log:?}",
            if native { "native" } else { "interp" }
        );
    }
}

/// The control that makes the cell above sound: a match that is not on `null` guards nothing.
///
/// `match x { 5 => … }` lowers to the same subject copy followed by `OpEqInt(subj, 5)`. The
/// null flows into that comparison as an ordinary operand and on into the program, so the
/// site still owes its report — and a version of the scan that accepted any test after the
/// copy would go quiet here.
#[test]
fn a_match_on_a_value_still_reports() {
    let source = "\
fn main() {
  v: vector<integer> = [7];
  x = v[5];
  r = match x { 5 => \"five\", _ => \"other\" };
  print(\"{r}\\n\");
}
";
    for native in [false, true] {
        let log = run_logged("match_value_reports", source, native);
        assert!(
            log.contains("[index_out_of_bounds]"),
            "a non-null match arm does not own the null and must still report ({}); got {log:?}",
            if native { "native" } else { "interp" }
        );
    }
}

/// The other direction, and the one that matters more: widening "guarded" SUPPRESSES a
/// diagnostic, so these three shapes must keep reporting.
///
/// Without them the scan above passes just as well if it stops requiring the check to be
/// about the faulting value at all — which would silence real faults rather than fix a
/// false one. Each cell breaks the guard differently: the null ESCAPES before the check,
/// the check is about ANOTHER variable, and the variable is REASSIGNED before the check.
#[test]
fn a_null_that_escapes_before_its_check_still_reports() {
    let source = "\
fn main() {
  tv: vector<text> = [\"a\"];
  u = tv[5];
  print(\"{u}\\n\");
  if u == null { print(\"n1\\n\"); }
  v2 = tv[5];
  g = 1;
  if g == 1 { print(\"n2\\n\"); }
  w = tv[5];
  w = \"x\";
  if w == null { print(\"no\\n\"); } else { print(\"n3\\n\"); }
}
";
    for native in [false, true] {
        let log = run_logged("escaped_null", source, native);
        let hits = log.matches("[index_out_of_bounds]").count();
        assert_eq!(
            hits,
            3,
            "a guard that does not own the null must not silence it ({}); got {log:?}",
            if native { "native" } else { "interp" }
        );
    }
}
