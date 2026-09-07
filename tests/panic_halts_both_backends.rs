// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `panic("msg")` must halt, report, and exit non-zero — on BOTH backends.
//!
//! It did not.  The native generator emits a real body only for the builtins it
//! special-cases; everything else with no `#rust` template falls through to
//! `fn n_x(..) {}` — an empty stub.  `n_panic` fell through, so on `--native`, which is
//! loft's DEFAULT backend, `panic()` printed nothing, halted nothing, and exited 0, while
//! `--interpret` printed the error and exited 1.  A four-line program was enough to show
//! it, and it silently defeated `loft-libs-net`'s `server::listen`, whose fatal-on-failed-
//! bind is the whole point of the call.
//!
//! Setting `had_fatal` the way the interpreter's `native.rs::n_panic` does is NOT enough
//! here: the generated `main` inspects that flag only after `n_main` returns, by which
//! point the program has run to completion past the panic.  Native has to report and exit
//! at the call site, which is what `RuntimeError::report_and_exit` does.
//!
//! Three separate properties, because each can regress alone and each hid part of the
//! original bug: the process STOPS, it exits NON-ZERO, and it SAYS why.
//!
//! The same stub-class also swallowed the whole `log_*` family — an audit of every
//! empty-bodied generated builtin turned up exactly two live cases, `panic` and the four
//! `log_*`, so both are guarded here.  (The other empty bodies are dead stubs whose calls
//! the generator lowers inline, and `yield_frame`, a documented native no-op.)

use std::path::PathBuf;
use std::process::Command;

fn run(backend: &str, src: &str, tag: &str) -> (i32, String, String) {
    let dir = std::env::temp_dir().join(format!("loft_panic_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("p.loft");
    std::fs::write(&path, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args([backend, path.to_str().unwrap()])
        .env("LOFT_TIMEOUT", "120")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("spawn loft");
    let _ = std::fs::remove_dir_all(&dir);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const SRC: &str = "fn main() {\n  println(\"before\");\n  panic(\"halt-marker\");\n  \
                   println(\"AFTER-PANIC\");\n}\n";

/// `rustc` is needed for the `--native` leg; skip cleanly where it is absent, like the
/// other native suites.
fn have_rustc() -> bool {
    Command::new("rustc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
        && PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/release")
            .exists()
}

#[test]
fn panic_halts_reports_and_exits_nonzero_on_both_backends() {
    let mut legs = vec![("--interpret", run("--interpret", SRC, "i"))];
    if have_rustc() {
        legs.push(("--native", run("--native", SRC, "n")));
    } else {
        println!("panic_halts...: --native leg skipped (no rustc)");
    }

    for (backend, (code, stdout, stderr)) in &legs {
        // 1. It STOPS.  This is the property that actually matters and the one that was
        //    broken: native ran the whole program, printing the line after the panic.
        assert!(
            !stdout.contains("AFTER-PANIC"),
            "[{backend}] execution continued past panic() — it is not halting.\n\
             stdout: {stdout:?}"
        );
        assert!(
            stdout.contains("before"),
            "[{backend}] the program did not run at all; this test is not measuring what \
             it thinks.\nstdout: {stdout:?}"
        );
        // 2. It exits NON-ZERO.  Native exited 0, so a script or CI step would have read
        //    a fatal panic as success.
        assert_ne!(
            *code, 0,
            "[{backend}] panic() exited 0 — a caller cannot tell it failed.\n\
             stderr: {stderr}"
        );
        // 3. It SAYS why.  An exit code with no message leaves nothing to act on.
        assert!(
            stderr.contains("halt-marker"),
            "[{backend}] the panic message never reached stderr.\nstderr: {stderr}"
        );
    }

    // Parity: the two backends must agree, not merely each be non-vacuous.  The renderer
    // is shared (`RuntimeError::to_diag_entry` + `render_entry_pretty`), so the reported
    // line is identical; comparing it is what pins the sharing in place.
    if legs.len() == 2 {
        let line_of = |s: &str| {
            s.lines()
                .find(|l| l.contains("panic:"))
                .unwrap_or("<no panic line>")
                .to_string()
        };
        let (i_code, _, i_err) = &legs[0].1;
        let (n_code, _, n_err) = &legs[1].1;
        assert_eq!(
            line_of(i_err),
            line_of(n_err),
            "backends render panic() differently"
        );
        assert_eq!(i_code, n_code, "backends disagree on panic()'s exit code");
    }
}

/// `assert` was NOT part of the same defect — it is special-cased in the generator and
/// always halted on native.  Pinned here so a future consolidation of the two builtins
/// cannot quietly regress the one that worked while fixing the one that did not.
/// loft#1405 — a write to a store the AUTHOR locked reaches them as a loft fault, not as
/// an internal assert.
///
/// `Store::read_only` is one flag for three different things — the const store, a worker
/// borrow, and the user's own `d#lock = true` — and only the last is a mistake an author
/// can make and fix.  All three used to abort through the same `assert!` in
/// `Store::addr_mut`, so a two-line loft program exited 101 with `panicked at
/// src/store.rs:2893`, the store's internal `rec`/`fld` numbers, and an invitation to set
/// `RUST_BACKTRACE` — an internal invariant reaching a user through a user-visible
/// feature, the shape loft#760 and loft#1262 also were.
///
/// `(H-WriteLocked)` was never violated: it says a locked write is "never a silent
/// successful write", and a panic is not silent.  What was missing is the fault CHANNEL,
/// which is why this is a diagnostic guard and not a rule one.
///
/// The same three properties as the tests above, plus a fourth this fault owns: both
/// backends must say it the SAME way.  The message deliberately carries no lock origin —
/// that spells an internal store number which differs per backend (`store_nr=2`
/// interpreted, `store_nr=0` native), and putting it in the text would make one event
/// render two ways.
#[test]
fn a_write_to_a_user_locked_store_is_a_loft_fault_on_both_backends() {
    let src = "struct C { val: integer }\n\
               fn main() {\n  println(\"before\");\n  d = C { val: 5 };\n  \
               d#lock = true;\n  d.val = 99;\n  println(\"AFTER-WRITE\");\n}\n";
    let mut legs = vec![("--interpret", run("--interpret", src, "li"))];
    if have_rustc() {
        legs.push(("--native", run("--native", src, "ln")));
    } else {
        println!("a_write_to_a_user_locked_store...: --native leg skipped (no rustc)");
    }

    let mut rendered: Vec<String> = Vec::new();
    for (backend, (code, stdout, stderr)) in &legs {
        // 1. It STOPS, and the program actually ran — without the second half a build
        //    that failed to start would pass every assertion below.
        assert!(
            !stdout.contains("AFTER-WRITE"),
            "[{backend}] execution continued past a write to a locked store.\n\
             stdout: {stdout:?}"
        );
        assert!(
            stdout.contains("before"),
            "[{backend}] the program did not run at all; this test is not measuring what \
             it thinks.\nstdout: {stdout:?}"
        );
        // 2. It exits NON-ZERO — and specifically NOT 101, which is what a Rust panic
        //    exits with.  101 here means the assert is back.
        assert_ne!(
            *code, 0,
            "[{backend}] a refused write exited 0.\nstderr: {stderr:?}"
        );
        assert_ne!(
            *code, 101,
            "[{backend}] exit 101 is a Rust panic, not a loft fault — the internal assert \
             in `Store::addr_mut` is reaching the author again.\nstderr: {stderr:?}"
        );
        // 3. It SAYS why, in loft's own voice and never in Rust's.
        assert!(
            stderr.contains("locked store"),
            "[{backend}] the fault does not name the locked store.\nstderr: {stderr:?}"
        );
        assert!(
            !stderr.contains("src/store.rs") && !stderr.contains("RUST_BACKTRACE"),
            "[{backend}] the author is being shown loft's internals.\nstderr: {stderr:?}"
        );
        rendered.push(stderr.clone());
    }
    // 4. Both backends say it the same way (loft#1058's property, and the reason the lock
    //    origin is not in the message).
    if rendered.len() == 2 {
        assert_eq!(
            rendered[0], rendered[1],
            "the two backends render one event differently"
        );
    }
}

#[test]
fn assert_still_halts_on_both_backends() {
    let src = "fn main() {\n  println(\"before\");\n  assert(1 == 2, \"assert-marker\");\n  \
               println(\"AFTER-ASSERT\");\n}\n";
    let mut legs = vec![("--interpret", run("--interpret", src, "ai"))];
    if have_rustc() {
        legs.push(("--native", run("--native", src, "an")));
    }
    for (backend, (code, stdout, stderr)) in &legs {
        assert!(
            !stdout.contains("AFTER-ASSERT"),
            "[{backend}] execution continued past a failed assert\nstdout: {stdout:?}"
        );
        assert_ne!(*code, 0, "[{backend}] failed assert exited 0");
        assert!(
            stderr.contains("assert-marker"),
            "[{backend}] the assert message never reached stderr.\nstderr: {stderr}"
        );
    }
}

/// `log_info` / `log_warn` / `log_error` / `log_fatal` must reach the log file on BOTH
/// backends.  They did not: a generated binary booted no logger at all, so every
/// structured log call was silently dropped on `--native` — measured at 2 records on
/// `--interpret` against 0 on `--native` with the same config.
///
/// Silent loss of diagnostics on the backend you actually deploy is the failure mode this
/// guards: nothing crashes, nothing is wrong in the output, the evidence just is not there.
///
/// Uses the DEFAULT config discovery (`log.conf` beside the program) rather than
/// `--log-conf`, deliberately — that flag goes to the loft driver, which a compiled binary
/// never sees, so a test built on it would pass on the interpreter and prove nothing about
/// native.  `LOFT_LOG_CONF` is the compiled-binary equivalent.
#[test]
fn log_family_writes_on_both_backends() {
    if !have_rustc() {
        println!("log_family_writes_on_both_backends: skipped (no rustc)");
        return;
    }
    let prog = "fn main() {\n  log_error(\"LOG-MARKER-E\");\n  log_warn(\"LOG-MARKER-W\");\n}\n";
    let conf = "[log]\nfile = log.txt\nlevel = warn\nproduction = false\n";

    let mut rendered = Vec::new();
    for (tag, backend) in [("li", "--interpret"), ("ln", "--native")] {
        let dir = std::env::temp_dir().join(format!("loft_logfam_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("p.loft"), prog).expect("write prog");
        std::fs::write(dir.join("log.conf"), conf).expect("write conf");
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .args([backend, dir.join("p.loft").to_str().unwrap()])
            .env("LOFT_TIMEOUT", "180")
            .current_dir(&dir)
            .output()
            .expect("spawn loft");
        let log = std::fs::read_to_string(dir.join("log.txt")).unwrap_or_default();
        assert!(
            log.contains("LOG-MARKER-E") && log.contains("LOG-MARKER-W"),
            "[{backend}] structured log records never reached log.txt — the log family is \
             a no-op on this backend.\nlog.txt: {log:?}\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        // Strip the timestamp, and reduce every path token to its BASENAME. The two
        // legs run in separate directories (so the native build artifacts can't
        // collide), and what must agree is severity, source basename + line, and
        // message. Reducing to the basename — rather than string-replacing the
        // constructed dir — is deliberate: on Windows the record renders the path in
        // a form that need not be byte-identical to `dir.to_string_lossy()` (8.3 short
        // names, separator/casing differences), so the old replace silently failed to
        // strip and the two legs never compared equal.
        let stripped: Vec<String> = log
            .lines()
            .map(|l| {
                l.split_whitespace()
                    .skip(1) // the leading timestamp token
                    .map(|tok| match tok.rsplit(['/', '\\']).next() {
                        // A path token (`<dir>/p.loft:2`) collapses to `p.loft:2`;
                        // a token with no separator (severity, message) is unchanged.
                        Some(base) if tok.contains(['/', '\\']) => base.to_string(),
                        _ => tok.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        rendered.push(stripped);
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert_eq!(
        rendered[0], rendered[1],
        "the backends write DIFFERENT log records for the same program"
    );
}

/// Production mode logs the fault and CONTINUES — on both backends (loft#1263).
///
/// `production = true` is documented to turn `panic()` into a fatal log entry and a failing
/// `assert()` into an error entry, with execution carrying on, "useful for long-running
/// services where a single error should not bring everything down"
/// (`DESIGN_DECISIONS.md § C66`).  It was implemented for the interpreter only: on
/// `--native` the panic aborted, the log stayed empty, and the statements after it never
/// ran — the exact outcome the feature exists to prevent, on the backend a user reaches by
/// typing `loft`.
///
/// The coverage that existed could not see it, and how is worth keeping: `runtime_logging.rs`
/// exercises production mode on `--interpret` only, and the rows above exercise both backends
/// with `production = false`.  Each axis was covered and the PAIR was not.
///
/// The halting rows above are this one's control.  Without them a `panic` that had become a
/// no-op again — which is what shipped once already, and is what the module header is about
/// — would satisfy every assertion here.
#[test]
fn production_mode_logs_and_continues_on_both_backends() {
    if !have_rustc() {
        println!("production_mode_logs_and_continues_on_both_backends: skipped (no rustc)");
        return;
    }
    let conf = "[log]\nfile = log.txt\nlevel = info\nproduction = true\n";
    // Both halting builtins, because they reach the same decision by different routes and
    // the native generator emits a separate body for each.
    for (what, prog, want_label) in [
        (
            "panic",
            "fn main() {\n  println(\"BEFORE\");\n  panic(\"boom\");\n  println(\"AFTER\");\n}\n",
            "[user_panic]",
        ),
        (
            "assert",
            "fn main() {\n  println(\"BEFORE\");\n  assert(1 == 2, \"bad\");\n  \
             println(\"AFTER\");\n}\n",
            "[assertion_failed]",
        ),
    ] {
        let mut rendered = Vec::new();
        for (tag, backend) in [("pi", "--interpret"), ("pn", "--native")] {
            let dir =
                std::env::temp_dir().join(format!("loft_prod_{}_{what}_{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(dir.join("p.loft"), prog).expect("write prog");
            std::fs::write(dir.join("log.conf"), conf).expect("write conf");
            let out = Command::new(env!("CARGO_BIN_EXE_loft"))
                .args([backend, dir.join("p.loft").to_str().unwrap()])
                .env("LOFT_TIMEOUT", "180")
                .current_dir(&dir)
                .output()
                .expect("spawn loft");
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let log = std::fs::read_to_string(dir.join("log.txt")).unwrap_or_default();

            // 1. It CONTINUED.  This is the property the feature exists for, and the one
            //    `--native` did not have.
            assert!(
                stdout.contains("AFTER"),
                "[{backend}/{what}] production must continue past the fault; \
                 stdout: {stdout:?}\nstderr: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            // 2. It SAID so.  An empty log file looks like a program with nothing to say.
            assert!(
                log.contains(want_label),
                "[{backend}/{what}] no {want_label} record reached the log — production \
                 mode was read and not honoured.\nlog: {log:?}"
            );
            // 3. The fault still COUNTED.  Production changes when the program stops, not
            //    whether the run failed.
            assert_eq!(
                out.status.code().unwrap_or(-1),
                1,
                "[{backend}/{what}] a logged fault must still exit non-zero"
            );

            // Same normalisation as `log_family_writes_on_both_backends`: drop the
            // timestamp and reduce every path token to its basename, since the two legs
            // run in separate directories.
            rendered.push(
                log.lines()
                    .map(|l| {
                        l.split_whitespace()
                            .skip(1)
                            .map(|tok| match tok.rsplit(['/', '\\']).next() {
                                Some(base) if tok.contains(['/', '\\']) => base.to_string(),
                                _ => tok.to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect::<Vec<_>>(),
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
        assert_eq!(
            rendered[0], rendered[1],
            "[{what}] the backends write DIFFERENT production records for the same program"
        );
    }
}
