// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The Logging chapter's claims, run against a real `log.conf`.
//!
//! Almost nothing in that chapter could be checked from inside it. Its cells call the four
//! log functions with no config present, where the documented behaviour is to do nothing —
//! so the chapter asserted that logging is off, and every claim about what logging DOES
//! (the record format, the level filter, per-file overrides, rate limiting, production
//! mode) ran under nothing at all. Three of them were wrong, and one of the three was the
//! config spelling the compiler's own `--generate-log-config` template prints.
//!
//! It cannot be a `.loft` cell either: each case needs its own `log.conf` beside its own
//! program, and a `log.conf` dropped into `tests/docs` would switch logging on for every
//! other chapter in the suite. So each case writes a private directory and runs the binary
//! in it.
//!
//! [`no_config_means_no_log_file`] is the control: without it, a build that wrote log lines
//! unconditionally would pass every positive case here.

use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// A private directory holding `app.loft` and (optionally) `log.conf`.
struct Case {
    dir: PathBuf,
}

impl Case {
    fn new(tag: &str, program: &str, conf: Option<&str>) -> Case {
        let dir = std::env::temp_dir().join(format!("loft_log_{tag}_{}", std::process::id()));
        let _ = fa::remove_dir_all(&dir);
        fa::create_dir_all(&dir).expect("create case dir");
        fa::write(dir.join("app.loft"), program).expect("write program");
        if let Some(c) = conf {
            fa::write(dir.join("log.conf"), c).expect("write log.conf");
        }
        Case { dir }
    }

    /// Run on `backend`; return `(stdout + stderr, log file contents)`.
    ///
    /// The log file is removed first: a logger APPENDS, so running one case on both
    /// backends otherwise reads the second run's records on top of the first's — which is
    /// how the record-format case first reported eight lines from four calls.
    fn run(&self, backend: &str) -> (String, String) {
        let _ = fa::remove_file(self.log_path());
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(self.dir.join("app.loft"))
            .env("LOFT_TIMEOUT", "300")
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("failed to invoke loft binary");
        let console = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let log = fa::read_to_string(self.dir.join("log.txt")).unwrap_or_default();
        (console, log)
    }

    fn log_path(&self) -> PathBuf {
        self.dir.join("log.txt")
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = fa::remove_dir_all(&self.dir);
    }
}

const FOUR_CALLS: &str = "fn main() {\n  \
                          log_info(\"an info line\");\n  \
                          log_warn(\"a warn line\");\n  \
                          log_error(\"an error line\");\n  \
                          log_fatal(\"a fatal line\");\n  \
                          println(\"done\");\n}\n";

const INFO_CONF: &str = "[log]\nfile = log.txt\nlevel = info\n";

/// The control. No `log.conf`, so nothing is configured and nothing is written — the state
/// the chapter's own cells run in, and the reason they could not check anything else.
#[test]
fn no_config_means_no_log_file() {
    let case = Case::new("noconf", FOUR_CALLS, None);
    for backend in ["--interpret", "--native"] {
        let (console, _) = case.run(backend);
        assert!(
            console.contains("done"),
            "[{backend}] the program must still run\n{console}"
        );
        assert!(
            !fa::exists(Path::new(&case.log_path())),
            "[{backend}] no log.conf must mean no log file"
        );
    }
}

/// The record format the chapter prints as an example: an ISO-8601 UTC timestamp to the
/// millisecond, a padded severity, `file:line`, then the message. The chapter showed
/// `2026-03-24 09:15:00` — a space, no milliseconds, no zone — which is what a reader would
/// have written their log parser against.
#[test]
fn a_record_carries_an_iso_timestamp_a_level_and_a_source_line() {
    let case = Case::new("format", FOUR_CALLS, Some(INFO_CONF));
    for backend in ["--interpret", "--native"] {
        let (_, log) = case.run(backend);
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 4, "[{backend}] four calls, four lines\n{log}");
        for (line, level) in lines.iter().zip(["INFO", "WARN", "ERROR", "FATAL"]) {
            // 2026-09-01T08:35:38.971Z — date, `T`, time, `.mmm`, `Z`.
            let stamp = &line[..24];
            assert!(
                stamp.len() == 24
                    && stamp.as_bytes()[10] == b'T'
                    && stamp.as_bytes()[23] == b'Z'
                    && stamp.as_bytes()[19] == b'.',
                "[{backend}] not an ISO-8601 millisecond stamp: {stamp:?}"
            );
            assert!(
                line[24..].trim_start().starts_with(level),
                "[{backend}] expected {level} in: {line}"
            );
            assert!(
                line.contains("app.loft:"),
                "[{backend}] no source location in: {line}"
            );
        }
        assert!(lines[0].ends_with("an info line"), "message: {}", lines[0]);
    }
}

/// `level` is a floor: everything below it is discarded.
#[test]
fn the_level_discards_everything_below_it() {
    let case = Case::new(
        "level",
        FOUR_CALLS,
        Some("[log]\nfile = log.txt\nlevel = error\n"),
    );
    let (_, log) = case.run("--interpret");
    assert!(
        !log.contains("an info line"),
        "info survived a floor of error\n{log}"
    );
    assert!(
        !log.contains("a warn line"),
        "warn survived a floor of error\n{log}"
    );
    assert!(log.contains("an error line"), "error was dropped\n{log}");
    assert!(log.contains("a fatal line"), "fatal was dropped\n{log}");
}

/// A `[levels]` key overrides the global floor for one file — in BOTH spellings.
///
/// The quoted one is what the chapter shows and what `loft --generate-log-config` prints
/// into its own template, and it did nothing: the parser stripped quotes from a value but
/// not from a key, so `"app.loft"` was stored with its quote characters and never matched
/// the bare basename it is looked up by. Silently, because an override that does nothing
/// looks exactly like one that was not needed. Both spellings are pinned so the asymmetry
/// cannot come back on either side.
#[test]
fn a_per_file_override_works_quoted_and_unquoted() {
    for (tag, key) in [("quoted", "\"app.loft\""), ("bare", "app.loft")] {
        let conf = format!("[log]\nfile = log.txt\nlevel = error\n\n[levels]\n{key} = info\n");
        let case = Case::new(&format!("override_{tag}"), FOUR_CALLS, Some(&conf));
        let (_, log) = case.run("--interpret");
        assert!(
            log.contains("an info line"),
            "[{tag}] the override did not lower the floor\n{log}"
        );
    }
}

/// …and it overrides in the other direction too, so it is a level rather than a switch.
#[test]
fn a_per_file_override_can_also_raise_the_floor() {
    let conf = "[log]\nfile = log.txt\nlevel = info\n\n[levels]\n\"app.loft\" = error\n";
    let case = Case::new("override_up", FOUR_CALLS, Some(conf));
    let (_, log) = case.run("--interpret");
    assert!(
        !log.contains("an info line"),
        "the override did not raise the floor\n{log}"
    );
    assert!(
        log.contains("an error line"),
        "error was dropped too\n{log}"
    );
}

/// `per_site` caps how many records one source LINE may write in a window.
#[test]
fn the_rate_limit_caps_one_source_line() {
    let program = "fn main() {\n  \
                   for i in 0..12 { log_warn(\"from one site {i}\"); }\n  \
                   println(\"done\");\n}\n";
    let conf = "[log]\nfile = log.txt\nlevel = info\n\n[rate_limit]\nper_site = 5\n";
    let case = Case::new("rate", program, Some(conf));
    let (_, log) = case.run("--interpret");
    let written = log.lines().filter(|l| l.contains("from one site")).count();
    assert_eq!(
        written, 5,
        "twelve calls, per_site = 5, got {written}\n{log}"
    );
}

/// A log message is an ordinary expression, evaluated before the call decides anything.
///
/// The chapter promised the opposite — "the string is never evaluated (no performance cost
/// for suppressed messages)" — which is the kind of claim a reader spends: an expensive
/// call inside a message on a hot path, believed free in production. It is evaluated in all
/// three suppressing cases, so all three are checked here.
#[test]
fn a_suppressed_message_is_still_evaluated() {
    let program = "fn noisy() -> integer { println(\"MESSAGE-EVALUATED\"); 7 }\n\
                   fn main() {\n  log_info(\"suppressed {noisy()}\");\n  println(\"done\");\n}\n";
    let cases: [(&str, Option<&str>); 3] = [
        ("eager_noconf", None),
        (
            "eager_floor",
            Some("[log]\nfile = log.txt\nlevel = error\n"),
        ),
        (
            "eager_ratelimited",
            Some("[log]\nfile = log.txt\nlevel = info\n\n[rate_limit]\nper_site = 0\n"),
        ),
    ];
    for (tag, conf) in cases {
        let case = Case::new(tag, program, conf);
        let (console, _) = case.run("--interpret");
        assert!(
            console.contains("MESSAGE-EVALUATED"),
            "[{tag}] the message expression must run even when the record is dropped\n{console}"
        );
    }
}

/// Production mode on the interpreter: `panic` logs FATAL and execution continues.
///
/// `--native` is NOT checked here and NOT because it agrees: it aborts and writes nothing
/// (loft#1263), and the default backend is the one that gets it wrong. Pinning that
/// divergence would be pinning the bug; the chapter names it instead.
#[test]
fn production_mode_turns_a_panic_into_a_fatal_record() {
    let program = "fn main() {\n  println(\"before\");\n  panic(\"deliberate\");\n  \
                   println(\"AFTER\");\n}\n";
    let conf = "[log]\nfile = log.txt\nlevel = info\nproduction = true\n";
    let case = Case::new("prod_panic", program, Some(conf));
    let (console, log) = case.run("--interpret");
    assert!(
        console.contains("AFTER"),
        "execution must continue past the panic\n{console}"
    );
    assert!(
        log.contains("FATAL") && log.contains("deliberate"),
        "the panic must reach the log as FATAL\n{log}"
    );
}

/// …and a failing `assert` becomes an ERROR record rather than an abort.
#[test]
fn production_mode_turns_a_failed_assert_into_an_error_record() {
    let program = "fn main() {\n  println(\"before\");\n  assert(1 == 2, \"deliberate\");\n  \
                   println(\"AFTER\");\n}\n";
    let conf = "[log]\nfile = log.txt\nlevel = info\nproduction = true\n";
    let case = Case::new("prod_assert", program, Some(conf));
    let (console, log) = case.run("--interpret");
    assert!(
        console.contains("AFTER"),
        "execution must continue past the failed assert\n{console}"
    );
    assert!(
        log.contains("ERROR") && log.contains("deliberate"),
        "the assertion must reach the log as ERROR\n{log}"
    );
}

/// A passing `assert` writes nothing — the chapter says so, and it is the difference between
/// a log you can read and one full of successes.
#[test]
fn a_passing_assert_logs_nothing() {
    let program = "fn main() {\n  assert(1 == 1, \"fine\");\n  println(\"done\");\n}\n";
    let case = Case::new("assert_ok", program, Some(INFO_CONF));
    let (console, log) = case.run("--interpret");
    assert!(console.contains("done"), "program must run\n{console}");
    assert!(log.is_empty(), "a passing assert wrote a record:\n{log}");
}
