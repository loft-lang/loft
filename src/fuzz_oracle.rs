// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @I90 — Shared utilities & data structures (test/fuzz harness support; @PLN53).
//
//! @PLN53 F1 — the mutational raw-source fuzz oracle, reified as a library
//! function so the code that is FUZZED is the code that is TESTED.
//!
//! [`fuzz_one_source`] drives one raw byte input through
//! `parse → byte_code → execute` in-process. The `program_source`
//! libfuzzer target is a two-line shim over it, and the seed-corpus replay
//! test drives the same function over the ~2000 `.loft` files under `cargo
//! test` on stable — no nightly, no cargo-fuzz.
//!
//! The pass/fail contract is pinned in
//! `doc/claude/plans/53-program-level-fuzzing/F1-DESIGN.md`. In one line: a run
//! ends **clean** on a `Diagnostics` rejection OR a loft-level `had_fatal`
//! fault, and **panics** if and only if the front-end hit an unhardened native
//! path — that panic IS the finding. The driver adds no panic surface of its
//! own (I/O and non-UTF-8 become clean returns) and swallows no language panic
//! (the panic-gate allowlist starts EMPTY).

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use crate::compile::byte_code;
use crate::data::Data;
use crate::database::Stores;
use crate::parser::Parser;
use crate::scopes;
use crate::state::State;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// The stdlib parsed once and cloned per input, so every run starts from an
/// identical fresh state (F1.0 site 3 — no cross-input state bleed).
static STDLIB: OnceLock<(Data, Stores)> = OnceLock::new();

/// The stdlib parsed once and cloned per input. Shared with the F2
/// keyed-container generator (`fuzz_keyed`) so both fuzzers pay the parse cost
/// once.
pub(crate) fn stdlib() -> (Data, Stores) {
    let (data, db) = STDLIB.get_or_init(|| {
        let mut p = Parser::new();
        // cwd is the repo root under `cargo test`, but `fuzz/` under cargo-fuzz.
        let dir = if std::path::Path::new("default").is_dir() {
            "default"
        } else {
            "../default"
        };
        p.parse_dir(dir, true, false)
            .expect("stdlib parse (run from the loft repo root or fuzz/)");
        (p.data, p.database)
    });
    (data.clone(), db.clone())
}

/// A per-call unique temp-file suffix, so concurrent test threads never share a
/// path regardless of input content (F1.0 site 1 — no harness-induced flake).
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// The clean terminal state a run reached. Every variant is NOT a finding — a
/// finding is a panic, which never returns an `Outcome` at all. The variants
/// exist so the seed-corpus replay can assert the pipeline actually *ran*
/// programs (not that it early-returned on everything — the vacuity trap F1.0
/// names): a corpus that only ever reaches `Rejected` is a broken harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Input was not valid UTF-8 (the lexer's contract is `&str`).
    NonUtf8,
    /// The temp-file write failed — a harness condition, not a language finding.
    IoError,
    /// The front-end rejected the program with a `Diagnostics` error. The
    /// common case: F1 feeds garbage.
    Rejected,
    /// The program compiled but had no `main`, so there was no entry to run.
    NoMain,
    /// The program compiled and `main` executed to completion (possibly ending
    /// in a loft-level `had_fatal` fault — correct on an arbitrary program).
    Ran,
}

/// Run one raw-source input through the full front-end and report a finding by
/// **panicking**; on any clean input, return which terminal [`Outcome`] it
/// reached.
///
/// This is the real oracle. [`fuzz_one_source`] is the libfuzzer shim over it.
/// It installs **no** `catch_unwind`, so a front-end panic propagates untouched
/// — the observer (the F1.2 test, the seed-corpus replay, or libfuzzer) decides
/// how to record it. Keeping the function a transparent conduit is exactly the
/// F1.0 invariant: a panic here means the language hit an unhardened path, not
/// the harness.
///
/// Poison off — F1's remit is front-end panics.
pub fn classify_source(src: &[u8]) -> Outcome {
    classify_source_with(src, false)
}

/// The oracle, with `poison` selecting F4's arena poison-on-free amplifier. On,
/// a store use-after-free reads a loud sentinel (typically a SIGSEGV) instead
/// of silently-lucky stale data — but that abort is uncatchable, so it is only
/// safe where crashes are isolated: the libfuzzer target (a crash = a recorded
/// artifact) runs it on; the in-process replay runs it off by default (one
/// uncatchable abort would kill the whole sweep) and opt-in via `LOFT_F1_POISON`.
/// Poison surfaces the store-lifetime UAF family (@PLN85 / the `program_ownership`
/// grammar), which is broader than F1's front-end remit.
pub fn classify_source_with(src: &[u8], poison: bool) -> Outcome {
    // Site 4: loft source is text; non-UTF-8 is not a language input.
    let Ok(text) = std::str::from_utf8(src) else {
        return Outcome::NonUtf8;
    };

    // Site 3: a fresh stdlib clone per call.
    let (data, db) = stdlib();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;

    // Parse via a temp FILE, not `parse_str`: the string path runs its own
    // two-pass reset discipline that the file path (the CLI/wrap path) does
    // not, so the file path is the faithful production entry.
    // Site 1: an I/O error is a harness condition, not a language finding.
    let tmp = std::env::temp_dir().join(format!(
        "loft_f1_{}_{}.loft",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    if std::fs::write(&tmp, text).is_err() {
        return Outcome::IoError;
    }
    let path = tmp.to_string_lossy().to_string();

    // Site 5: no panic gate — a parser panic on malformed input is the finding.
    p.parse(&path, false);
    let _ = std::fs::remove_file(&tmp);

    // A graceful diagnostic rejection is the common, clean case (F1 feeds
    // garbage). Warning/Debug lines are not rejections.
    let rejected = !p.diagnostics.is_empty()
        && p.diagnostics.lines().iter().any(|l| {
            // "not a warning" means "an error" here, so a coded diagnostic that no
            // prefix match recognised used to read as a REJECTION (@PLN131).
            !matches!(
                crate::diagnostics::compact_level(l),
                Some(crate::diagnostics::Level::Warning | crate::diagnostics::Level::Debug)
            )
        });
    if rejected {
        return Outcome::Rejected;
    }

    // No `main` — no entry point to run. Parse already exercised its panic
    // surface; still compile (a scopes/byte_code ICE on an accepted program is
    // a finding), then stop rather than fault on a missing entry.
    let has_main = p.data.def_nr("n_main") < p.data.definitions();

    // Compile. Still no panic gate: an ICE here is a finding.
    let mut data = p.data;
    let mut database = p.database;
    scopes::check(&mut data, &mut database);
    let mut state = State::new(database);
    byte_code(&mut state, &mut data);
    if !has_main {
        return Outcome::NoMain;
    }

    // Arena poison-on-free (F4): a stale read becomes loud garbage rather than
    // silently lucky data. Off by default (see `classify_source_with`).
    state.database.poison_free = poison;
    state.execute("main", &data);
    while state.database.frame_yield {
        state.resume();
    }

    // The F1 flip: `had_fatal` is a loft-level fault (a `raise`, a failed
    // `assert`, an index-out-of-bounds) — correct behaviour on an arbitrary
    // program, so NOT a finding. Only a Rust panic/abort above is.
    let _ = state.database.had_fatal;
    Outcome::Ran
}

/// The libfuzzer entry point: drive one input and discard the clean outcome. A
/// finding still propagates as a panic (or a poison-induced abort), which
/// libfuzzer records with the crashing artifact. Poison is ON here — the
/// fuzzer isolates and records each crash, so the store-UAF amplifier is pure
/// upside.
pub fn fuzz_one_source(src: &[u8]) {
    let _ = classify_source_with(src, true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    /// The observer the F1.2 checks and the F1.3 replay share: wrap
    /// `classify_source` so a finding (panic) is caught as `Err` instead of
    /// aborting the test binary. `classify_source` installs no `catch_unwind`
    /// of its own (visible in its body), so any front-end panic reaches this
    /// same `Err` path — the observer here is the whole finding channel.
    fn observe(src: &[u8]) -> Result<Outcome, ()> {
        // No global panic-hook swap here: clean inputs never panic (so no
        // spam), and a real finding's backtrace is exactly what we want to see.
        // Swapping the process-global hook per call would race the rest of the
        // parallel suite (and fire 2000× under the F1.3 replay).
        catch_unwind(AssertUnwindSafe(|| classify_source(src))).map_err(|_| ())
    }

    // ── (a) the FINDING side — prove the harness CAN fail ───────────────────

    /// The observer reports a panic as a finding — not a silent pass, not a
    /// test-binary abort. This is the same `catch_unwind` the replay uses; with
    /// `classify_source` transparent, a real front-end panic lands here too.
    #[test]
    fn observer_reports_a_panic_as_finding() {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let caught = catch_unwind(AssertUnwindSafe(|| panic!("planted F1 finding")));
        std::panic::set_hook(hook);
        assert!(caught.is_err(), "a panic must be observed as a finding");
    }

    // ── (b) the CLEAN side — the common cases are not findings ──────────────

    #[test]
    fn garbage_bytes_are_rejected_not_a_finding() {
        let garbage = b")(*&^%$ not loft fn fn {{{ 123 ";
        assert_eq!(observe(garbage), Ok(Outcome::Rejected));
    }

    #[test]
    fn non_utf8_is_clean() {
        assert_eq!(observe(&[0xff, 0xfe, 0x00, 0x80]), Ok(Outcome::NonUtf8));
    }

    // ── (c) NON-VACUITY — the pipeline actually compiles and runs ───────────

    #[test]
    fn valid_program_runs_to_completion() {
        let src = b"fn main() { x: integer = 1 + 2; }";
        assert_eq!(observe(src), Ok(Outcome::Ran));
    }

    #[test]
    fn definitions_without_main_compile_but_do_not_run() {
        let src = b"fn helper() -> integer { 7 }";
        assert_eq!(observe(src), Ok(Outcome::NoMain));
    }

    /// The F1 flip, proven empirically: a program whose `main` faults at
    /// RUNTIME (here an out-of-bounds index) reaches `Ran` — a `had_fatal`
    /// fault is correct behaviour on an arbitrary program, NOT a finding.
    #[test]
    fn runtime_fault_is_clean_not_a_finding() {
        let src = b"fn main() { v: vector<integer> = []; y: integer = v[5]; }";
        assert_eq!(observe(src), Ok(Outcome::Ran));
    }

    /// A fault the front-end catches at COMPILE time is a `Diagnostics` rejection, not
    /// a run — the boundary between `Rejected` and a runtime `Ran` fault.  A constant
    /// `1 / 0` is `integer?` (`(N-Div)`), and storing it into a NARROW `u8` is the
    /// `(N-Store)` error, since a `u8` has no bit pattern left for null; into a
    /// full-width `integer` it is a warning and the program RUNS (@PLN153 phase 3b), so
    /// the narrow slot is the one that still draws the line this cell measures.
    #[test]
    fn compile_time_fault_is_rejected() {
        let src = b"fn main() { x: u8 = 1 / 0; }";
        assert_eq!(observe(src), Ok(Outcome::Rejected));
    }

    // ── F1.3 — seed-corpus replay ───────────────────────────────────────────

    use std::path::{Path, PathBuf};

    /// Like `observe`, but recover the panic message for triage.
    fn observe_msg(src: &[u8], poison: bool) -> Result<Outcome, String> {
        catch_unwind(AssertUnwindSafe(|| classify_source_with(src, poison))).map_err(|e| {
            e.downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "<non-string panic payload>".to_string())
        })
    }

    fn collect_loft(root: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(root) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_loft(&p, out);
            } else if p.extension().is_some_and(|x| x == "loft") {
                out.push(p);
            }
        }
    }

    /// An *environment* condition, not a language finding: a program that calls
    /// a native-extension function whose cdylib is not built in the fuzz
    /// context (or an integration test needing external resources). Keyed on the
    /// message so it survives file moves.
    fn is_environment(first_line: &str) -> bool {
        crate::compile::is_native_not_loaded(first_line)
    }

    /// Triaged panics — each an entry in the @PLN53 F1 findings catalogue
    /// (`doc/claude/plans/53-program-level-fuzzing/README.md`). A panic that is
    /// neither an environment condition nor listed here is an UNTRIAGED finding
    /// and fails the replay. `(path-substring, message-substring, catalogue-id)`.
    const TRIAGED: &[(&str, &str, &str)] = &[
        // F1-1 is FIXED (unary-prefix operators now run `known_var_or_type`, so
        // an undefined operand is a clean diagnostic, not a codegen panic) —
        // `unary_minus.loft` now reaches `Rejected`, so no allowlist entry.
        //
        // Harness artifact, NOT a language bug: an index-out-of-bounds that
        // fires only under the preloaded-stdlib-cache parse path, not a fresh
        // CLI parse (the same clone-cache parse asymmetry `program_ownership`
        // documents). Recorded in the catalogue as a harness limitation.
        (
            "51-hidden-buffer-aliasing/probes/51-tuple-as-arg.loft",
            "index out of bounds",
            "F1-harness-cache",
        ),
    ];

    /// Replay `classify_source` over every `.loft` file in the repo. Each file
    /// is a valid seed regardless of whether it is a runnable program: any
    /// `Outcome` (a lib module → `Rejected`/`NoMain`, an intentional-error
    /// fixture → `Rejected`, a real program → `Ran`) is clean. Only a PANIC is
    /// a finding. `#[ignore]` (heavy) — run with:
    ///   cargo test --lib fuzz_oracle::tests::seed_corpus_replay -- --ignored --nocapture
    #[test]
    #[ignore = "F1 seed-corpus replay — heavy; runs nightly in miri.yml's `release-gate-sweeps` job; by hand: `cargo test --release --lib fuzz_oracle::tests::seed_corpus_replay -- --ignored`"]
    fn seed_corpus_replay() {
        // On the `loft` CLI's own stack — 8 MiB, or `RUST_MIN_STACK` when a sanitizer job
        // raises it — as `tests/wrap.rs` runs the corpus: the front end spends ~22 KiB of
        // stack per nesting level of an expression, so a test thread's 2 MiB overflowed on
        // the 100-term sum in `a-frame-never-pushes-past-the-room-its-entry-made.loft`,
        // which the CLI runs (loft#1863).  A panic re-raises here, failing the test as before.
        let stack = std::env::var("RUST_MIN_STACK")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .map_or(8 << 20, |v| v.max(8 << 20));
        let run = std::thread::Builder::new()
            .stack_size(stack)
            .spawn(replay_seed_corpus)
            .expect("spawn the replay thread");
        if let Err(panic) = run.join() {
            std::panic::resume_unwind(panic);
        }
    }

    fn replay_seed_corpus() {
        use std::fmt::Write as _;
        let base = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        for root in ["tests", "doc", "examples", "lib"] {
            collect_loft(&base.join(root), &mut files);
        }
        files.sort();
        assert!(
            files.len() > 500,
            "corpus too small ({}) — file discovery is broken",
            files.len()
        );

        // Collect the panic messages ourselves; silence the per-file backtrace
        // flood for the duration, then restore.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let mut findings: Vec<(PathBuf, String)> = Vec::new();
        let mut ran = 0usize;
        let mut env_skipped = 0usize;
        let mut triaged = 0usize;
        let trace = std::env::var_os("LOFT_F1_TRACE").is_some();
        // Poison (F4 store-UAF amplifier) is opt-in: on, an uncatchable abort
        // kills the whole in-process sweep, so it is for the isolated libfuzzer
        // target. Default off keeps the replay on F1's front-end-panic remit.
        let poison = std::env::var_os("LOFT_F1_POISON").is_some();
        for f in &files {
            let Ok(bytes) = std::fs::read(f) else {
                continue;
            };
            if trace {
                // Flushed before the call: an uncatchable abort (SIGSEGV / stack
                // overflow) leaves this as the last line, naming the culprit.
                eprintln!("TRY {}", f.strip_prefix(base).unwrap_or(f).display());
            }
            match observe_msg(&bytes, poison) {
                Ok(Outcome::Ran) => ran += 1,
                Ok(_) => {}
                Err(msg) => {
                    let first = msg.lines().next().unwrap_or("");
                    let path = f.to_string_lossy();
                    if is_environment(first) {
                        env_skipped += 1;
                    } else if TRIAGED
                        .iter()
                        .any(|(p, m, _)| path.contains(p) && first.contains(m))
                    {
                        triaged += 1;
                    } else {
                        findings.push((f.clone(), msg));
                    }
                }
            }
        }
        std::panic::set_hook(hook);

        eprintln!(
            "F1.3 replay: {} files, {ran} Ran, {env_skipped} env-skipped, \
             {triaged} triaged-known, {} NEW finding(s)",
            files.len(),
            findings.len()
        );

        // Non-vacuity at corpus scale: the pipeline genuinely ran real programs
        // — not an all-`Rejected` harness that would swallow every finding.
        assert!(
            ran > 100,
            "only {ran} files reached Ran — the harness is likely broken"
        );

        // Any NEW panic — not an environment condition, not a triaged-known
        // entry — is an untriaged finding and fails loudly (F1.0 site 5: the
        // allowlist grows only through explicit, referenced `TRIAGED` entries).
        if !findings.is_empty() {
            let mut report = format!("F1 seed-corpus replay: {} finding(s)\n", findings.len());
            for (p, m) in &findings {
                let rel = p.strip_prefix(base).unwrap_or(p);
                let first = m.lines().next().unwrap_or("");
                let _ = writeln!(report, "  {}\n      {first}", rel.display());
            }
            panic!("{report}");
        }
    }
}
