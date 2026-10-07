// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-07 phase 0 — error-message golden corpus.
//!
//! Each `tests/error_messages/cases/<NN_name>.loft` (or directory with a
//! `main.loft` inside) is run through the compiled `loft` binary in
//! `--interpret` mode; the captured stdout+stderr+exit-code is compared
//! byte-for-byte against `tests/error_messages/baseline/<NN_name>.expect`.
//!
//! Native mode is intentionally NOT used here — it brings in `rustc` plus
//! the cargo build cache (zip-crate rmeta/rlib pairing in this checkout)
//! which currently emits an environment-specific error unrelated to the
//! diagnostic surface we want to measure.  The interpreter path is the
//! ground-truth language reference and is what plan-07 phase 1+ will
//! improve against this baseline.
//!
//! Re-capture all baselines:
//!     UPDATE_GOLDEN=1 cargo test --release --test error_messages
//!
//! Suppress suggestions (phase 5):
//!     LOFT_NO_SUGGESTIONS=1 cargo test --release --test error_messages
//!
//! Cases that contain a directory (multi-file scenarios such as
//! `39_import_circular/`) treat `<dir>/main.loft` as the entry point and
//! pass `--lib <dir>` so sibling files resolve.

use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn cases_dir() -> PathBuf {
    workspace_root().join("tests/error_messages/cases")
}

fn baseline_dir() -> PathBuf {
    workspace_root().join("tests/error_messages/baseline")
}

/// Entry shape for one case.  Either a single `.loft` file or a directory
/// containing `main.loft` plus optional siblings.
struct Case {
    name: String,             // e.g. "01_parse_unclosed_paren"
    entry: PathBuf,           // .loft file actually passed to loft
    lib_dir: Option<PathBuf>, // for multi-file cases, --lib <dir>
}

fn collect_cases() -> Vec<Case> {
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(cases_dir())
        .expect("read cases dir")
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let file_name = e.file_name().to_string_lossy().into_owned();
        if fa::is_file(&path) && file_name.ends_with(".loft") {
            let name = file_name.trim_end_matches(".loft").to_string();
            out.push(Case {
                name,
                entry: path,
                lib_dir: None,
            });
        } else if fa::is_dir(&path) {
            let entry = path.join("main.loft");
            if fa::exists(&entry) {
                out.push(Case {
                    name: file_name,
                    entry,
                    lib_dir: Some(path),
                });
            }
        }
    }
    out
}

/// Replace anything that varies between machines or runs:
///   * absolute path prefix to the case file → "<cases>/<name>.loft"
///   * absolute workspace-root prefix → "<workspace>"
///   * /tmp/loft_native scratch paths → "<tmp>/loft_native"
///   * `thread 'NAME' (PID)` → `thread 'NAME' (<pid>)` (Rust panic header)
///   * `<file>.rs:NNN:NNN` from the loft *crate's* sources →
///     `<file>.rs:<line>:<col>` (panic origin moves on every edit;
///     the test's contract is the panic *content*, not the line in
///     loft's internal `panic_fmt` site).  Only crate-internal
///     paths (`src/...`) are scrubbed; the case-file path remains
///     literal because the user-visible loft source line IS part
///     of the contract.
fn normalise(raw: &str, case_name: &str, entry: &Path) -> String {
    // Windows: `std::fs::canonicalize` returns `\\?\C:\...` UNC paths
    // and the OS uses backslash separators throughout.  Strip the UNC
    // prefix and convert all backslashes to forward slashes so the
    // baselines stay platform-neutral.  Loft's panic output never
    // includes literal `\n`/`\t` escapes (panics produce real
    // newlines), so this aggressive conversion is safe for the case
    // corpus.
    let mut s = raw.replace(r"\\?\", "").replace('\\', "/");
    let entry_abs = entry.to_string_lossy().replace('\\', "/");
    let entry_rel = format!("<cases>/{case_name}.loft");
    let workspace = workspace_root().to_string_lossy().replace('\\', "/");
    s = s.replace(&entry_abs, &entry_rel);
    s = s.replace(&workspace, "<workspace>");
    // Also catch the directory form for multi-file cases.
    if let Some(parent) = entry.parent() {
        let parent_abs = parent.to_string_lossy().replace('\\', "/");
        if parent_abs != workspace {
            s = s.replace(&parent_abs, &format!("<cases>/{case_name}"));
        }
    }
    // Native scratch dir (only present if a case slips into native mode).
    s = s.replace("/tmp/loft_native", "<tmp>/loft_native");
    // The stale-rlib advisory is about THIS CHECKOUT's build order, not about the
    // program: `target/release/libloft.rlib` is older than `deps/libloft.rlib`
    // after any `cargo test` build, so a case that builds a library emits it here
    // and not on a machine that last ran `cargo build --lib`. Capturing it would
    // pin one build order into the baseline and make the case fail on the other.
    s = s
        .lines()
        .filter(|l| !l.starts_with("loft: warning — ") || !l.contains("is STALE"))
        .collect::<Vec<_>>()
        .join("\n");
    if raw.ends_with('\n') && !s.ends_with('\n') {
        s.push('\n');
    }
    // PID in Rust panic header: `thread 'main' (12345)` → `thread 'main' (<pid>)`.
    s = scrub_thread_pid(&s);
    // Crate-internal source line numbers in Rust panic origins.
    s = scrub_crate_source_locations(&s);
    // Strip Rust panic stack backtraces.  CI sets RUST_BACKTRACE=1 by
    // default for diagnostics; locally it's usually unset.  The
    // backtrace frames are environment-dependent (rustc internal
    // paths, frame ordering varies by build) and aren't part of the
    // test contract.  `run_case` already calls env_remove() but this
    // is a defence-in-depth — even if a future code path leaks
    // RUST_BACKTRACE, the comparison still works.
    s = strip_stack_backtrace(&s);
    // Registry auto-install diagnostics are ENVIRONMENT-DEPENDENT and not part
    // of the error under test.  A `use unknown_module` first triggers an
    // auto-install attempt; its failure line differs by network — offline it is
    // a DNS error (`… Dns Failed: … Temporary failure in name resolution`),
    // online a registry not-found, and it is absent entirely on a machine with a
    // warm registry.  The contract the case asserts is the "Library not found"
    // diagnostic that follows.  Strip every `[registry] …` line so the baseline
    // is deterministic across CI, offline dev, and the sandbox.
    s = strip_registry_diagnostics(&s);
    s
}

/// Remove `[registry] …` lines (network-dependent auto-install diagnostics).
/// Mirrors `strip_stack_backtrace`'s trailing-newline handling.
fn strip_registry_diagnostics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for line in s.lines() {
        if line.trim_start().starts_with("[registry]") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !s.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Remove "stack backtrace:" + everything until the next blank line.
/// Some Rust panics include the backtrace inline before the
/// "note: run with `RUST_BACKTRACE=1`" line.  This strip is robust
/// to both forms.
fn strip_stack_backtrace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut iter = s.lines().peekable();
    while let Some(line) = iter.next() {
        if line.trim_start().starts_with("stack backtrace:") {
            // Skip until we hit a blank line OR the
            // "note: run with `RUST_BACKTRACE`..." line OR end of input.
            for skip in iter.by_ref() {
                let t = skip.trim();
                if t.is_empty() || t.starts_with("note: ") {
                    out.push_str(skip);
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    // Preserve trailing newline shape: if input had one, keep it; if
    // not, drop the synthetic one we added.
    if !s.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Replace `src/<path>.rs:NNN:NNN` with `src/<path>.rs:<line>:<col>`.
/// Catches Rust panic origin lines in the loft crate; preserves
/// case-file lines (`<cases>/foo.loft:42:10`) which are the actual
/// user-visible contract.
fn scrub_crate_source_locations(s: &str) -> String {
    // Match `src/...rs:NNN:NNN` and `src/...rs:NNN`.
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find("src/") {
        out.push_str(&rest[..idx]);
        let tail = &rest[idx..];
        // Find ".rs:" to confirm this is a Rust file reference.
        if let Some(rs_idx) = tail.find(".rs:") {
            let (path_part, after) = tail.split_at(rs_idx + ".rs:".len());
            // after starts with "NNN" or "NNN:NNN".  Consume digits +
            // optional ":NNN".  Anything else stays literal.
            let after_bytes = after.as_bytes();
            let mut i = 0;
            while i < after_bytes.len() && after_bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i > 0 {
                // Has digits — scrub them.
                out.push_str(path_part);
                out.push_str("<line>");
                let mut j = i;
                if j < after_bytes.len() && after_bytes[j] == b':' {
                    let mut k = j + 1;
                    while k < after_bytes.len() && after_bytes[k].is_ascii_digit() {
                        k += 1;
                    }
                    if k > j + 1 {
                        out.push_str(":<col>");
                        j = k;
                    }
                }
                rest = &after[j..];
                continue;
            }
        }
        // No match — copy "src/" and continue.
        out.push_str(&tail[..4]);
        rest = &tail[4..];
    }
    out.push_str(rest);
    out
}

/// Replace the numeric PID inside `thread 'NAME' (NNN)` panic headers.
fn scrub_thread_pid(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find("thread '") {
        out.push_str(&rest[..idx]);
        rest = &rest[idx..];
        // find the closing `' (`
        if let Some(close) = rest.find("' (") {
            // copy through the `' (`
            out.push_str(&rest[..close + 3]);
            rest = &rest[close + 3..];
            // skip the digits
            let digits_end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            if digits_end > 0 && rest[digits_end..].starts_with(')') {
                out.push_str("<pid>");
                rest = &rest[digits_end..];
            }
        } else {
            // no close — copy the rest and stop
            out.push_str(rest);
            return out;
        }
    }
    out.push_str(rest);
    out
}

fn run_case(case: &Case) -> String {
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret");
    if let Some(dir) = &case.lib_dir {
        cmd.arg("--lib").arg(dir);
    }
    cmd.arg(&case.entry).current_dir(workspace_root());
    // Strip RUST_BACKTRACE so CI runners (which set it to 1 for
    // diagnostics) don't get a different output than local runs.
    // The test contract is the panic message, not the backtrace
    // frames.
    cmd.env_remove("RUST_BACKTRACE");
    let out = cmd.output().expect("invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let exit = out
        .status
        .code()
        .map_or("signal".to_string(), |c| c.to_string());
    let raw = format!(
        "==== exit: {exit} ====\n\
         ==== stdout ====\n{stdout}\
         ==== stderr ====\n{stderr}"
    );
    normalise(&raw, &case.name, &case.entry)
}

// @speed 3.0
#[test]
fn baselines_are_locked_in() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let cases = collect_cases();
    assert!(
        cases.len() >= 40,
        "expected at least 40 cases, found {}",
        cases.len()
    );
    let mut diffs: Vec<String> = Vec::new();
    let baseline = baseline_dir();
    fa::create_dir_all(&baseline).expect("create baseline dir");
    for case in &cases {
        let actual = run_case(case);
        let expect_path = baseline.join(format!("{}.expect", case.name));
        if update {
            fa::write(&expect_path, &actual).expect("write golden");
            continue;
        }
        let expected = fa::read_to_string(&expect_path).unwrap_or_default();
        if expected != actual {
            diffs.push(format!(
                "--- {} ---\nexpected:\n{}\nactual:\n{}\n",
                case.name, expected, actual
            ));
        }
    }
    if !diffs.is_empty() {
        panic!(
            "{} baseline mismatch(es):\n{}\n\
             Re-capture with:  UPDATE_GOLDEN=1 cargo test --release --test error_messages",
            diffs.len(),
            diffs.join("\n")
        );
    }
}

// @speed 2.5
#[test]
fn every_case_terminates_cleanly() {
    let cases = collect_cases();
    for case in &cases {
        // `run_case` blocks until the child exits.  If a case hung
        // forever, this test would never finish — so passing this test
        // is itself the no-infinite-loop assertion.  We additionally
        // require that no case crashes via SIGSEGV / SIGABRT (signal
        // exit), which would mask a regression.
        let mut cmd = Command::new(loft_bin());
        cmd.arg("--interpret");
        if let Some(dir) = &case.lib_dir {
            cmd.arg("--lib").arg(dir);
        }
        cmd.arg(&case.entry).current_dir(workspace_root());
        // Same RUST_BACKTRACE strip as `run_case`.
        cmd.env_remove("RUST_BACKTRACE");
        let out = cmd.output().expect("invoke loft binary");
        if out.status.code().is_none() {
            panic!(
                "case {} terminated by signal (likely SIGSEGV / SIGABRT); \
                 stderr={}",
                case.name,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
