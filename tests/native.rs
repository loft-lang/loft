// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The `--native` (C backend) integration tests.
//!
//! The corpus tests run each `.loft` file through the `loft` binary the way a user does —
//! `loft --tests <file> --native` for a file of test functions, `loft --native <file>` for a
//! script-shaped program — so the program is compiled to C, built with the host C compiler
//! and run.  The rest drive the C binding (`#c`), packaged C libraries and shipped programs
//! through the same binary.  They do **not** acquire `WRAP_LOCK`: a corpus program's run
//! holds [`common::source_run_lock`], which serialises it only against another run of the
//! SAME file (the interpreter's, in `wrap`).

extern crate loft;

use loft::file_access as fa;
use std::io::Error;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
mod common;

/// Process-wide mutex serialising this binary's native tests.
///
/// Each corpus test already runs [`c_lanes`] builds at once; two such pools in one process
/// (`cargo test` runs tests as threads) would hold the CPU twice over.  Across processes —
/// nextest runs each test in its own — the share is `threads-required` in
/// `.config/nextest.toml` instead (`pool_tests_declare_their_lanes_to_nextest`).
fn native_suite_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Files in `tests/docs/` that `native_dir` must not compile.
///
/// Empty, and staying empty is the point: every page the site publishes is a
/// program that runs on both backends, so an entry here is a page whose code a
/// reader cannot trust.  Add one only with the open issue that explains it, and
/// delete it the day that issue closes.
const NATIVE_SKIP: &[&str] = &[];

/// Script files in `tests/scripts/` that the C backend does not run.
///
/// Empty: an entry names a file the C backend cannot carry, the open issue that explains it,
/// and the condition that removes it.
const SCRIPTS_NATIVE_SKIP: &[&str] = &[];

/// Every `LOFT_X=v` on a `// @ARMED:` line: the run-time instruments a guard's defect is only
/// visible under.  A stale address reads the old bytes until the allocator reuses the freed
/// block, so a plain run of a use-after-free guard passes on the broken build too, while
/// `LOFT_POISON=1` overwrites that block.  The language suite's runner reads the same line.
fn armed_switches(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in src.lines() {
        let Some(rest) = line.trim().strip_prefix("// @ARMED:") else {
            continue;
        };
        for tok in rest.split_whitespace() {
            if let Some((k, v)) = tok.split_once('=')
                && k.starts_with("LOFT_")
                && k.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                && !v.is_empty()
            {
                out.push((k.to_owned(), v.to_owned()));
            }
        }
    }
    out
}

#[test]
fn armed_switches_reads_the_header_line_only() {
    let src = "// @ARMED: LOFT_POISON=1 LOFT_STRICT_STORES=1\n\
               //   ARMED: LOFT_HOIST_VERIFY=1 — a receipt's label, not an arm\n\
               // @ARMED: lower=1 LOFT_X= LOFT_TEXT_TIMELINE=1\n";
    assert_eq!(
        armed_switches(src),
        [
            ("LOFT_POISON", "1"),
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_TEXT_TIMELINE", "1")
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
    );
}

/// The environment a corpus program's annotations ask for, set on the `loft` run that
/// compiles it and so inherited by the binary it builds: `// @LIMIT:` and `// @FRAME_LIMIT:`
/// (the lowered compile limits, `LOFT_LIMIT` / `LOFT_FRAME_LIMIT`), `// @STORE_LIMIT:` (the
/// run's store ceiling, `LOFT_STORE_LIMIT`) and the `// @ARMED:` switches.
fn program_env(src: &str) -> Vec<(String, String)> {
    let header = |key: &str| {
        src.lines()
            .take(20)
            .find_map(|l| l.trim().strip_prefix(key))
            .and_then(|v| v.trim().parse::<u32>().ok())
    };
    let mut env = Vec::new();
    for (key, var) in [
        ("// @LIMIT:", "LOFT_LIMIT"),
        ("// @FRAME_LIMIT:", "LOFT_FRAME_LIMIT"),
    ] {
        if let Some(n) = header(key) {
            env.push((var.to_owned(), n.to_string()));
        }
    }
    if let Some(n) = loft::limits::Limits::store_limit_annotation(src) {
        env.push(("LOFT_STORE_LIMIT".to_owned(), n.to_string()));
    }
    env.extend(armed_switches(src));
    env
}

/// How a corpus file runs on the C backend.
#[derive(Clone, Copy)]
enum CRun {
    /// `loft --tests <file> --native`: the file's test functions, its `@ARGS` and its
    /// `@EXPECT_*` annotations read by the test runner.
    Tests,
    /// `loft --native <file>`: the file as a program, the way a user runs it.
    Program,
}

/// A `// @SCRIPT` file has top-level statements and no test functions: it runs as a program.
fn script_shaped(src: &str) -> bool {
    src.lines().any(|l| l.starts_with("// @SCRIPT"))
}

/// Build and run `entry` on the C backend; `Err` carries the run's stdout and stderr.
///
/// Under `LOFT_NATIVE_LEAK_CHECK` a store left at exit fails the program, as it fails a
/// script on the interpreter: the binary reports it on stderr with exit 0.
fn run_on_c(entry: &Path, how: CRun) -> Result<(), String> {
    let src = fa::read_to_string(entry).map_err(|e| format!("{}: {e}", entry.display()))?;
    let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
    match how {
        CRun::Tests => cmd.arg("--tests").arg(entry).arg("--native"),
        CRun::Program => cmd.arg("--native").arg(entry),
    };
    cmd.env("LOFT_TIMEOUT", "300");
    for (k, v) in program_env(&src) {
        cmd.env(k, v);
    }
    let _run_lock = common::source_run_lock(entry);
    let out = cmd
        .output()
        .map_err(|e| format!("{}: could not start loft: {e}", entry.display()))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let leaked = std::env::var_os("LOFT_NATIVE_LEAK_CHECK").is_some()
        && stderr.contains("stores not freed at program exit");
    if out.status.success() && !leaked {
        return Ok(());
    }
    Err(format!(
        "{} ({}{}):\n{}{stderr}",
        entry.display(),
        out.status,
        if leaked { ", stores left at exit" } else { "" },
        String::from_utf8_lossy(&out.stdout),
    ))
}

/// How many C builds a corpus test runs at once: the cores, at most 8, and under nextest the
/// share `.config/nextest.toml` declares for it (`common::lanes`).
fn c_lanes(jobs: usize) -> usize {
    common::lanes::lanes(8).min(jobs.max(1))
}

/// Run every `(file, how)` on the C backend across [`c_lanes`] threads, then fail with every
/// failure — one file's failure does not hide another's.
fn run_all_on_c(jobs: &[(PathBuf, CRun)]) -> std::io::Result<()> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failures: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..c_lanes(jobs.len()) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((entry, how)) = jobs.get(i) else {
                        break;
                    };
                    let began = std::time::Instant::now();
                    let verdict = run_on_c(entry, *how);
                    println!(
                        "  {:6.1}s  {} {}",
                        began.elapsed().as_secs_f64(),
                        if verdict.is_ok() { "ok  " } else { "FAIL" },
                        entry.display()
                    );
                    if let Err(e) = verdict {
                        failures
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .push((i, e));
                    }
                }
            });
        }
    });
    let mut failures = failures.into_inner().unwrap_or_else(|p| p.into_inner());
    failures.sort_by_key(|(i, _)| *i);
    println!(
        "\nC result: {} passed, {} failed; {} total",
        jobs.len() - failures.len(),
        failures.len(),
        jobs.len()
    );
    if failures.is_empty() {
        return Ok(());
    }
    for (_, e) in &failures {
        println!("\n{e}");
    }
    Err(Error::other(format!(
        "{} of {} programs failed on the C backend: {}",
        failures.len(),
        jobs.len(),
        failures
            .iter()
            .map(|(i, _)| jobs[*i].0.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// The sorted `.loft` files of `dir`; `None` when the directory is absent.
fn loft_files(dir: &str) -> Option<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = fa::read_dir(dir)
        .ok()?
        .into_iter()
        .map(|e| e.os_spelling())
        .filter(|p| fa::extension(p).is_some_and(|e| e.eq_ignore_ascii_case("loft")))
        .collect();
    files.sort();
    Some(files)
}

/// Build and run every `.loft` file in `tests/docs/` on the C backend, skipping files listed
/// in `NATIVE_SKIP`.  A file that cannot be built or whose run fails is a failure: a suite
/// that reported a pass for a program it never ran would be worse than a red one.
#[test]
fn native_dir() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut jobs = Vec::new();
    for entry in loft_files("tests/docs").ok_or_else(|| Error::other("no tests/docs"))? {
        let name = fa::file_name(&entry).unwrap_or_default();
        if NATIVE_SKIP.contains(&name.as_str()) {
            println!("skip {entry:?} (native skip list — see NATIVE_SKIP)");
            continue;
        }
        let src = fa::read_to_string(&entry)?;
        let how = if script_shaped(&src) {
            CRun::Program
        } else {
            CRun::Tests
        };
        jobs.push((entry, how));
    }
    run_all_on_c(&jobs)
}

/// Build and run every `.loft` file in `tests/reference/` on the C backend — the reference
/// pages' samples, the `--native` half of `wrap::reference`.  An `@EXPECT_ERROR` file is
/// skipped (the refusal is the interpreter's to prove); a `// @SCRIPT` file runs as a program.
#[test]
fn native_reference() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let Some(files) = loft_files("tests/reference") else {
        return Ok(());
    };
    let mut jobs = Vec::new();
    for entry in files {
        let src = fa::read_to_string(&entry)?;
        if src.contains("@EXPECT_ERROR") {
            println!("skip {entry:?} (expected error — the interpreter proves the refusal)");
            continue;
        }
        let how = if script_shaped(&src) {
            CRun::Program
        } else {
            CRun::Tests
        };
        jobs.push((entry, how));
    }
    run_all_on_c(&jobs)
}

/// Compile and run every code sample of `doc/learn-loft.md` natively — the `--native` half of
/// `wrap::learn_loft_samples` (loft#1809).
#[test]
fn native_learn_loft_samples() {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let failed: Vec<String> = common::learn_loft_samples()
        .iter()
        .filter_map(|s| common::check_learn_sample(s, "--native", "300").err())
        .collect();
    assert!(failed.is_empty(), "{}", failed.join("\n\n"));
}

/// Build and run every `.loft` file in `tests/comparisons/` on the C backend — the comparison
/// pages' claims, the `--native` half of `wrap::comparisons`.  Every file there is
/// script-shaped (`// @SCRIPT`), so each runs as a program.
#[test]
fn native_comparisons() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let Some(files) = loft_files("tests/comparisons") else {
        return Ok(());
    };
    let mut jobs = Vec::new();
    for entry in files {
        let src = fa::read_to_string(&entry)?;
        let how = if script_shaped(&src) {
            CRun::Program
        } else {
            CRun::Tests
        };
        jobs.push((entry, how));
    }
    run_all_on_c(&jobs)
}

/// Build and run every extracted feature example in `tests/docs/features/` on the C backend —
/// the native half of the features catalogue's "example-must-run" guard (@I81).  These files
/// are generated from `loft-lang/features` issues by `tools/features/gen.loft`; only
/// complete-program examples land here.  An example that imports a library (a `use` line)
/// runs as a program, `loft --native <file>`, the way its reader would run it.  A no-op when
/// the directory is absent.
#[test]
fn native_features() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let Some(files) = loft_files("tests/docs/features") else {
        return Ok(());
    };
    let mut jobs = Vec::new();
    for entry in files {
        let src = fa::read_to_string(&entry)?;
        // Selected by the `use` RULE, not by filename, so the next library example is
        // covered without an edit here.
        let imports_library = src.lines().any(|l| l.trim_start().starts_with("use "));
        let how = if imports_library || script_shaped(&src) {
            CRun::Program
        } else {
            CRun::Tests
        };
        jobs.push((entry, how));
    }
    run_all_on_c(&jobs)
}

/// Build and run one chunk of the `.loft` files in `tests/scripts/` on the C backend,
/// skipping files listed in `SCRIPTS_NATIVE_SKIP`.
///
/// CHUNKED: `native_scripts_00` … take every [`NATIVE_SCRIPT_CHUNKS`]-th script of the sorted
/// corpus from their own offset, so each script is in exactly one chunk and each chunk grows
/// with 1/N of the corpus; a chunk is held to `common::timing::CHUNK_BUDGET`.  Each script is
/// its own binary, so no cross-script coverage is lost by splitting.
fn native_scripts_chunk(chunk: usize) -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let files: Vec<PathBuf> = loft_files("tests/scripts")
        .ok_or_else(|| Error::other("no tests/scripts"))?
        .into_iter()
        .enumerate()
        .filter(|(i, _)| i % NATIVE_SCRIPT_CHUNKS == chunk)
        .map(|(_, p)| p)
        .collect();
    native_scripts_run(
        files,
        common::timing::Suite::start(
            &format!("native_scripts_{chunk:02}"),
            common::timing::CHUNK_BUDGET,
        ),
    )
}

/// The limit guards at the real limits (`tests/limits_full/`) on `--native`.  The per-commit
/// corpus proves the same relations under LOWERED limits (`// @LIMIT:`, `// @FRAME_LIMIT:`),
/// in milliseconds; compiling a program 300 levels deep costs time that grows with the square
/// of the depth.  Nightly, beside `wrap::limits_at_full_size`.
#[test]
#[ignore = "the limit guards at the real 300 / 64 KiB — miri.yml's nightly `release-gate-sweeps` job; run with --ignored"]
fn native_limits_at_full_size() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let files =
        loft_files("tests/limits_full").ok_or_else(|| Error::other("no tests/limits_full"))?;
    native_scripts_run(
        files,
        common::timing::Suite::unbounded("native_limits_at_full_size"),
    )
}

/// Build and run `files` on the C backend with the script corpus's skip rules, reported
/// through `suite` as one batch.
///
/// A file that is not meant to run says so — `@EXPECT_ERROR`, a file-level `@EXPECT_FAIL`,
/// or a row in `SCRIPTS_NATIVE_SKIP` with its reason; every other file that fails to build
/// or run is a failure.  A fn-level `@EXPECT_FAIL` names one function, which the test runner
/// skips; its siblings still run.
fn native_scripts_run(files: Vec<PathBuf>, suite: common::timing::Suite) -> std::io::Result<()> {
    let mut jobs = Vec::new();
    for entry in files {
        let name = fa::file_name(&entry).unwrap_or_default();
        if SCRIPTS_NATIVE_SKIP.contains(&name.as_str()) {
            println!("skip {entry:?} (scripts native skip list — see SCRIPTS_NATIVE_SKIP)");
            continue;
        }
        let src = fa::read_to_string(&entry)?;
        // Read the annotation the way `wrap` reads it (`common::expect_tag`), not with a
        // `contains` over the whole source: a file that merely NAMES the tag in prose
        // declares nothing.
        if common::declares_expect_error(&src) {
            println!("skip {entry:?} (has @EXPECT_ERROR)");
            continue;
        }
        if common::expect_fail_fns(&src).1 {
            println!("skip {entry:?} (has file-level @EXPECT_FAIL)");
            continue;
        }
        let how = if script_shaped(&src) {
            CRun::Program
        } else {
            CRun::Tests
        };
        jobs.push((entry, how));
    }
    let batch = format!("C build + run, {} programs", jobs.len());
    let unit = suite.begin(&batch);
    let ran = run_all_on_c(&jobs);
    suite.end_parallel(&batch, unit, c_lanes(jobs.len()));
    ran?;
    suite.finish()
}

/// How many tests the native script corpus is split across — see [`native_scripts_chunk`].
const NATIVE_SCRIPT_CHUNKS: usize = 64;

macro_rules! native_script_chunks {
    ($($name:ident = $k:expr),* $(,)?) => {
        $(
            #[test]
            fn $name() -> std::io::Result<()> {
                native_scripts_chunk($k)
            }
        )*
    };
}

native_script_chunks!(
    native_scripts_00 = 0,
    native_scripts_01 = 1,
    native_scripts_02 = 2,
    native_scripts_03 = 3,
    native_scripts_04 = 4,
    native_scripts_05 = 5,
    native_scripts_06 = 6,
    native_scripts_07 = 7,
    native_scripts_08 = 8,
    native_scripts_09 = 9,
    native_scripts_10 = 10,
    native_scripts_11 = 11,
    native_scripts_12 = 12,
    native_scripts_13 = 13,
    native_scripts_14 = 14,
    native_scripts_15 = 15,
    native_scripts_16 = 16,
    native_scripts_17 = 17,
    native_scripts_18 = 18,
    native_scripts_19 = 19,
    native_scripts_20 = 20,
    native_scripts_21 = 21,
    native_scripts_22 = 22,
    native_scripts_23 = 23,
    native_scripts_24 = 24,
    native_scripts_25 = 25,
    native_scripts_26 = 26,
    native_scripts_27 = 27,
    native_scripts_28 = 28,
    native_scripts_29 = 29,
    native_scripts_30 = 30,
    native_scripts_31 = 31,
    native_scripts_32 = 32,
    native_scripts_33 = 33,
    native_scripts_34 = 34,
    native_scripts_35 = 35,
    native_scripts_36 = 36,
    native_scripts_37 = 37,
    native_scripts_38 = 38,
    native_scripts_39 = 39,
    native_scripts_40 = 40,
    native_scripts_41 = 41,
    native_scripts_42 = 42,
    native_scripts_43 = 43,
    native_scripts_44 = 44,
    native_scripts_45 = 45,
    native_scripts_46 = 46,
    native_scripts_47 = 47,
    native_scripts_48 = 48,
    native_scripts_49 = 49,
    native_scripts_50 = 50,
    native_scripts_51 = 51,
    native_scripts_52 = 52,
    native_scripts_53 = 53,
    native_scripts_54 = 54,
    native_scripts_55 = 55,
    native_scripts_56 = 56,
    native_scripts_57 = 57,
    native_scripts_58 = 58,
    native_scripts_59 = 59,
    native_scripts_60 = 60,
    native_scripts_61 = 61,
    native_scripts_62 = 62,
    native_scripts_63 = 63,
);

/// Every chunk index has its test: a chunk count raised without a matching test would drop that
/// slice of the corpus from every run, silently.
#[test]
fn native_script_chunks_cover_the_corpus() {
    let names = [
        "native_scripts_00",
        "native_scripts_01",
        "native_scripts_02",
        "native_scripts_03",
        "native_scripts_04",
        "native_scripts_05",
        "native_scripts_06",
        "native_scripts_07",
        "native_scripts_08",
        "native_scripts_09",
        "native_scripts_10",
        "native_scripts_11",
        "native_scripts_12",
        "native_scripts_13",
        "native_scripts_14",
        "native_scripts_15",
        "native_scripts_16",
        "native_scripts_17",
        "native_scripts_18",
        "native_scripts_19",
        "native_scripts_20",
        "native_scripts_21",
        "native_scripts_22",
        "native_scripts_23",
        "native_scripts_24",
        "native_scripts_25",
        "native_scripts_26",
        "native_scripts_27",
        "native_scripts_28",
        "native_scripts_29",
        "native_scripts_30",
        "native_scripts_31",
        "native_scripts_32",
        "native_scripts_33",
        "native_scripts_34",
        "native_scripts_35",
        "native_scripts_36",
        "native_scripts_37",
        "native_scripts_38",
        "native_scripts_39",
        "native_scripts_40",
        "native_scripts_41",
        "native_scripts_42",
        "native_scripts_43",
        "native_scripts_44",
        "native_scripts_45",
        "native_scripts_46",
        "native_scripts_47",
        "native_scripts_48",
        "native_scripts_49",
        "native_scripts_50",
        "native_scripts_51",
        "native_scripts_52",
        "native_scripts_53",
        "native_scripts_54",
        "native_scripts_55",
        "native_scripts_56",
        "native_scripts_57",
        "native_scripts_58",
        "native_scripts_59",
        "native_scripts_60",
        "native_scripts_61",
        "native_scripts_62",
        "native_scripts_63",
    ];
    assert_eq!(
        names.len(),
        NATIVE_SCRIPT_CHUNKS,
        "one generated test per chunk"
    );
    let src = fa::read_to_string(file!()).expect("read tests/native.rs");
    for n in names {
        assert!(src.contains(&format!("{n} = ")), "{n} is generated");
    }
}

/// Every test that spawns a worker pool declares its lanes to nextest, and the two numbers
/// are ONE number (@PLN193 P0).  `common::lanes::UNDER_NEXTEST` sizes the pool; the
/// `threads-required` override in `.config/nextest.toml` is what nextest counts against the
/// run's thread budget — a pool-spawning test missing from that override runs its workers
/// beside a full pool again, which is the multiplication that hung the box at 184 threads.
#[test]
fn pool_tests_declare_their_lanes_to_nextest() {
    let toml = fa::read_to_string(".config/nextest.toml").expect("read .config/nextest.toml");
    let want = format!("threads-required = {}", common::lanes::UNDER_NEXTEST);
    // The override block that carries `threads-required`: from its `[[profile.…` header to
    // the next header.
    let blocks: Vec<&str> = toml.split("[[profile.").collect();
    let block = blocks
        .iter()
        .find(|b| b.lines().any(|l| l.trim() == want))
        .unwrap_or_else(|| panic!("no override in .config/nextest.toml carries `{want}`"));
    assert!(
        block.starts_with("default.overrides]]"),
        "the `{want}` override must be a profile.default override, so every profile inherits it"
    );
    let filter = block
        .lines()
        .find_map(|l| l.trim().strip_prefix("filter = "))
        .expect("the threads-required override has a filter");
    // Every pool-spawning test, by the pattern its filter must contain.
    for needle in [
        "native_scripts_[0-9]+",
        "native_dir",
        "native_reference",
        "native_learn_loft_samples",
        "native_comparisons",
        "native_features",
        "native_limits_at_full_size",
        "every_cell_releases_each_resource_once_on_native_[0-9]+",
        "every_cell_releases_each_resource_once_on_the_interpreter",
        "the_census_names_the_copy_each_cell_makes",
        "a_refused_cell_is_a_compile_error_and_a_once_cell_is_not",
        "every_emitted_copy_of_a_droppable_has_a_lease_verdict",
        "the_zero_on_claim_census_only_shrinks",
        "features_examples_interpret",
    ] {
        assert!(
            filter.contains(needle),
            "`{needle}` spawns a worker pool but is not in the threads-required filter:\n{filter}"
        );
    }
}

/// The POSIX `write(2)` binding as the generated sources below spell it.
const POSIX_WRITE: &str = r#"#c "write" "long(int, const void*, size_t)""#;

/// Spell a generated `#c` source for THIS host. A no-op off Windows.
///
/// Windows has `write(2)`'s behaviour but not its NAME: the CRT exports it as
/// `_write`, so a declaration naming `write` resolves to nothing there and the
/// call faults — measured, `` `#c` symbol 'write' not found ``.
///
/// Only the symbol is branched, because only the symbol differs. `long` is
/// genuinely right for the return on both: POSIX `write` gives `ssize_t`
/// (64-bit on LP64) and `_write` gives `int` (32-bit), which is exactly what C
/// `long` means on each — one of the places where naming the platform's own
/// width is the correct binding rather than a portability bug. The count
/// narrows to `unsigned int` for the same reason: that is `_write`'s third
/// parameter, where POSIX takes `size_t`.
fn for_host(src: &str) -> String {
    if loft::platform::host_lib_os() == loft::platform::LibOs::Windows {
        src.replace(
            POSIX_WRITE,
            r#"#c "_write" "long(int, const void*, unsigned int)""#,
        )
    } else {
        src.to_string()
    }
}

/// A `#c` binding calls a real C symbol from a `--native` (C) program, with no wrapper.
///
/// Bound to **libc**, deliberately: every program links it, so this proves the whole path —
/// declaration, typed C prototype, marshalling, call — with no build step and nothing to
/// install.
///
/// `atoi("-1")` is the cell that matters. A C `int` return read back as a bare `u64` — what
/// a signature-blind caller does — is 4294967295, a plausible large positive that every
/// `>= 0` check accepts; it is the shape that silently defeated `loft-libs-net`'s
/// `server::listen`. It comes back as -1 only because the declared width reaches the call,
/// so the result is truncated at the ABI and then sign-extended.
///
/// `write(1, ptr, count)` covers the other half: a loft `vector` crosses as a pointer AND a
/// count, because C carries no length.
#[test]
fn native_c_binding_calls_libc() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let path = std::env::temp_dir().join("loft_pln24_c_binding.loft");
    fa::write(
        &path,
        for_host(
            "pub fn c_strlen(s: text) -> integer;   #c \"strlen\" \"size_t(const char*)\"\n\
         pub fn c_atoi(s: text) -> integer;     #c \"atoi\" \"int(const char*)\"\n\
         pub fn c_abs(v: integer) -> integer;   #c \"abs\" \"int(int)\"\n\
         pub fn c_write(fd: integer, v: vector<u8>) -> integer;  #c \"write\" \"long(int, const void*, size_t)\"\n\
         fn main() {\n\
         \x20 println(\"len {c_strlen(\"hello\")}\");\n\
         \x20 println(\"neg {c_atoi(\"-1\")}\");\n\
         \x20 println(\"abs {c_abs(-7)}\");\n\
         \x20 b: vector<u8> = [];\n\
         \x20 for ch in \"hi\\n\" { b += [(ch as integer) as u8? ?? (0 as u8)] }\n\
         \x20 println(\"wrote {c_write(1, b)}\");\n\
         }\n",
        ),
    )?;
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--native")
        .arg(&path)
        .env("LOFT_TIMEOUT", "300")
        .output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("len 5"), "strlen: {stdout}");
    assert!(
        stdout.contains("neg -1"),
        "a 32-bit C return must sign-extend, not come back as 4294967295 — the \
         declared width is what makes that work: {stdout}"
    );
    assert!(stdout.contains("abs 7"), "abs: {stdout}");
    // `\r\n` -> `\n` before comparing. The bytes really do differ: the C runtime
    // opens fd 1 in TEXT mode on Windows, so the `write(1, "hi\n", 3)` this test
    // makes arrives as `hi\r\n`. Measured with `{stdout:?}` after the CI log —
    // which normalises line endings — showed output that looked identical to a
    // passing run. The translation is the platform behaving as documented, not
    // the binding losing a byte, and the claim under test is that the vector
    // crossed as pointer + count, which `wrote 3` is what settles.
    let stdout = stdout.replace("\r\n", "\n");
    assert!(
        stdout.contains("hi\n") && stdout.contains("wrote 3"),
        "a vector must cross as pointer + count: {stdout:?}"
    );
    Ok(())
}

/// The interpreter calls the same C symbols as the C backend, and answers the SAME thing.
///
/// The interpreter has no compiler at the call site, so it resolves the symbol
/// and calls it through the fixed per-arity trampolines the architecture probe
/// validated. This test is the parity half: identical stdout from both
/// backends, cell for cell, which is the bar a `#c` binding has to meet before
/// it can be said to work at all.
///
/// The `neg` cell carries the whole reason the declaration has a signature: the
/// interpreter reads a raw `u64` return register, so `atoi("-1")` arrives as
/// 4294967295 unless the DECLARED width truncates and re-extends it.
#[test]
fn interpreted_and_native_c_bindings_agree() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let path = std::env::temp_dir().join("loft_pln24_c_parity.loft");
    fa::write(
        &path,
        for_host(
            "pub fn c_strlen(s: text) -> integer;   #c \"strlen\" \"size_t(const char*)\"\n\
         pub fn c_atoi(s: text) -> integer;     #c \"atoi\" \"int(const char*)\"\n\
         pub fn c_abs(v: integer) -> integer;   #c \"abs\" \"int(int)\"\n\
         pub fn c_write(fd: integer, v: vector<u8>) -> integer;  #c \"write\" \"long(int, const void*, size_t)\"\n\
         pub fn c_memcmp(a: text, b: text, n: integer) -> integer;  #c \"memcmp\" \"int(const void*, const void*, size_t)\"\n\
         fn main() {\n\
         \x20 println(\"past {c_memcmp(\"ab\\0cd\", \"ab\\0ce\", 5) != 0} {c_memcmp(\"ab\\0cd\", \"ab\\0cd\", 5)}\");\n\
         \x20 println(\"len {c_strlen(\"hello\")}\");\n\
         \x20 println(\"neg {c_atoi(\"-1\")}\");\n\
         \x20 println(\"abs {c_abs(-7)}\");\n\
         \x20 b: vector<u8> = [];\n\
         \x20 for ch in \"hi\\n\" { b += [(ch as integer) as u8? ?? (0 as u8)] }\n\
         \x20 println(\"wrote {c_write(1, b)}\");\n\
         }\n",
        ),
    )?;
    let interp = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg(&path)
        .output()?;
    let out = String::from_utf8_lossy(&interp.stdout).into_owned();
    let err = String::from_utf8_lossy(&interp.stderr);
    assert!(interp.status.success(), "interpret failed: {out}\n{err}");
    assert!(out.contains("len 5"), "{out}");
    assert!(
        out.contains("neg -1"),
        "the interpreter reads a RAW return register — without the declared \
         width this is 4294967295: {out}"
    );
    assert!(out.contains("abs 7") && out.contains("wrote 3"), "{out}");
    // A text holds a length and may hold a NUL; the crossing hands C every byte of it and
    // then one NUL, so a callee that takes the byte count beside the pointer reads past an
    // interior NUL. `strlen` on that pointer stops there: a library that hashes or matches
    // a text takes `size(t)` as an argument instead.
    assert!(
        out.contains("past true 0"),
        "the bytes after an interior NUL must cross with the text: {out}"
    );

    // The parity half, on the C backend.
    let native = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--native")
        .arg(&path)
        .env("LOFT_TIMEOUT", "300")
        .output()?;
    let nout = String::from_utf8_lossy(&native.stdout);
    assert!(
        native.status.success(),
        "--native failed: {nout}\n{}",
        String::from_utf8_lossy(&native.stderr)
    );
    assert_eq!(
        out, nout,
        "the two backends must answer identically, cell for cell"
    );
    Ok(())
}

/// @PLN24 arc D — the composition matrix, against a REAL C library, on both
/// backends.
///
/// Arcs A-C proved the mechanism against libc, which is already in the process.
/// This is the shape a library actually has: a package that declares
/// `[c] libs`, a `.so` built by nothing but `cc`, and a binding per cell of the
/// plan's mapping table — every integer width, a text argument, a vector as
/// pointer + count, the opaque-handle open/read/bump/close cycle, and the
/// 7-argument call that straddles the SysV register/stack boundary.
///
/// The assertion is that the two backends produce **identical** output, because
/// that is the only claim worth making: each one alone can be plausibly wrong
/// in a way the other is not.
///
/// Skips when `cc` is absent — the fixture is C, and a machine without a C
/// compiler cannot build it. It does NOT skip silently on a failed build: that
/// is a real failure.
#[test]
fn c_binding_matrix_against_a_declared_library() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi");
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let built = loft::platform::process::harness_command("make")
        .arg("-C")
        .arg(&root)
        .output()?;
    assert!(
        built.status.success(),
        "the fixture library must build: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let prog = std::env::temp_dir().join("loft_pln24_matrix.loft");
    fa::write(
        &prog,
        "use lcabi::*;\n\
         fn main() {\n\
         \x20 println(\"i64 {lc_i64(lc_i64(1234567890123))}\");\n\
         \x20 println(\"neg {lc_neg_i32(1)}\");\n\
         \x20 println(\"len {lc_strlen(\"loft\")}\");\n\
         \x20 println(\"byte {lc_byte_at(\"loft\", 0)}\");\n\
         \x20 v: vector<integer> = [10, 20, 30];\n\
         \x20 println(\"vec {lc_i64_sum(v)}\");\n\
         \x20 h = lc_open(1000);\n\
         \x20 println(\"handle {lc_read(h)} {lc_bump(h, 7)}\");\n\
         \x20 lc_close(h);\n\
         \x20 println(\"arity7 {lc_arity7(1,1,1,1,1,1,1)}\");\n\
         \x20 t = lc_static_text();\n\
         \x20 println(\"text {t} {t.len()}\");\n\
         \x20 println(\"textarg {lc_strlen(lc_static_text())}\");\n\
         \x20 println(\"cat [{lc_static_text()}][{lc_static_text()}]\");\n\
         \x20 println(\"some {lc_maybe_text(1)}\");\n\
         \x20 println(\"none {lc_opt_text(0) ?? \"<null>\"}\");\n\
         \x20 println(\"latin1 {lc_latin1_text().len()}\");\n\
         \x20 c_text_positions();\n\
         }\n\
         // A text return has to reach EVERY value position, because the caller\n\
         // needs a destination record handed to it and only some positions were\n\
         // routed at first: a struct literal, a vector literal, a field write, a\n\
         // `match` subject and a comparison all lower to `w += producer()`, which\n\
         // is a different emission site from a plain local assignment. Missing it\n\
         // is not a slow path for a `#c` binding — there is no non-destination\n\
         // sibling to fall back to, so it SIGSEGV'd the interpreter while\n\
         // `--native` stayed correct. One function per position, so a regression\n\
         // names the position it broke.\n\
         struct CRec { name: text, n: integer }\n\
         fn c_text_positions() {\n\
         \x20 o = CRec{name: lc_static_text(), n: 1};\n\
         \x20 println(\"p-lit {o.name}\");\n\
         \x20 o.name = lc_maybe_text(1);\n\
         \x20 println(\"p-field {o.name}\");\n\
         \x20 if lc_static_text() == \"loft/c-abi\" { println(\"p-cmp ok\") } else { println(\"p-cmp NO\") }\n\
         \x20 m = match lc_static_text() { \"loft/c-abi\" => lc_maybe_text(1), _ => \"-\" };\n\
         \x20 println(\"p-match {m}\");\n\
         \x20 v: vector<text> = [lc_static_text(), lc_maybe_text(1)];\n\
         \x20 println(\"p-vlit {v[0]} {v[1]}\");\n\
         \x20 println(\"p-cond {if lc_static_text().len() > 3 { lc_static_text() } else { \"-\" }}\");\n\
         \x20 for i in 0..2 { println(\"p-loop{i} {lc_static_text()}\") }\n\
         \x20 s = lc_static_text() + \"!\";\n\
         \x20 println(\"p-concat {s}\");\n\
         }\n",
    )?;
    let libdir = root.join("pkg");
    let run = |backend: &str| -> std::io::Result<String> {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // The EXIT STATUS belongs in the message, because the interesting Windows
        // failure is a SILENT one: a binary that links but cannot find its DLL at
        // load time dies with `STATUS_DLL_NOT_FOUND` (0xC0000135) having written
        // nothing at all, so stdout and stderr are both empty and the assertion
        // said nothing without this.
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    // Hand-computed, not copied from a run: `lc_i64` is self-inverse, the
    // vector sum is position-weighted (10*1 + 20*2 + 30*3), and arity7's
    // arguments carry distinct prime weights (2+3+5+7+11+13+17).
    for want in [
        "i64 1234567890123",
        "neg -1",
        "len 4",
        "byte 108",
        "vec 140",
        "handle 1000 1007",
        "arity7 58",
        // @PLN24 arc D — the `char *` return. Hand-computed: "loft/c-abi" is 10
        // characters, so C's byte count and loft's character count agree on it
        // (`textarg` re-crosses the answer to prove the copy is NUL-terminated,
        // not merely non-empty). NULL reads as loft null. "caf\xE9" is 3 ASCII
        // bytes plus one invalid UTF-8 byte, which becomes ONE replacement
        // character — 4, not 3 (dropped) and not 5 (bytes counted as characters).
        "text loft/c-abi 10",
        "textarg 10",
        "cat [loft/c-abi][loft/c-abi]",
        "some here",
        "none <null>",
        "latin1 4",
        // Every value position, each named so a failure says which one broke.
        "p-lit loft/c-abi",
        "p-field here",
        "p-cmp ok",
        "p-match here",
        "p-vlit loft/c-abi here",
        "p-cond loft/c-abi",
        "p-loop0 loft/c-abi",
        "p-loop1 loft/c-abi",
        "p-concat loft/c-abi!",
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    let native = run("--native")?;
    assert_eq!(
        interp, native,
        "the two backends must agree cell for cell against a real library"
    );
    Ok(())
}

/// @PLN128 arc C / C106 — the `#c` arity ceiling is ONE contract, and both
/// backends are held to it.
///
/// This is a regression guard for an asymmetry that was silent for a long time:
/// the check ran behind `if !native_mode`, so `--native` bound and correctly
/// CALLED a 14-slot C function while the interpreter refused the same
/// declaration. A library author could ship that and only a downstream consumer
/// would find out — `loft debug` is the interpreter, so the bindings you could
/// not debug were exactly the ones with no other way in.
///
/// Both halves of the boundary are pinned, because either one alone can pass
/// while the contract is broken: at `MAX_C_ARITY` both backends must CALL and
/// agree on the value, and one past it both must REFUSE. The expected sums are
/// position-weighted (argument `i` counts `i`), so a trampoline that dropped or
/// reordered an argument gives a different number rather than a plausible one.
/// @C106 — one `#c` arity ceiling for both backends.
#[test]
fn the_c_arity_ceiling_is_the_same_on_both_backends() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let max = loft::c_signature::MAX_C_ARITY;
    let dir = std::env::temp_dir().join("loft_pln128_arity");
    let src = dir.join("pkg/arity/src");
    fa::create_dir_all(&src)?;

    // One C function at the ceiling and one past it, each weighting argument i
    // by i+1 so a dropped or reordered argument gives a different number rather
    // than a plausible one.
    let mut csrc = String::new();
    for n in [max, max + 1] {
        let params = (0..n)
            .map(|i| format!("long long a{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let body = (0..n)
            .map(|i| format!("a{i}*{}", i + 1))
            .collect::<Vec<_>>()
            .join(" + ");
        csrc.push_str(&format!("long long ar{n}({params}) {{ return {body}; }}\n"));
    }
    fa::write(dir.join("arity.c"), &csrc)?;
    // The manifest keeps the LINUX spelling on every host: `platform::lib_variants`
    // translates `libarity.so` to `arity.dll` for Windows and `libarity.dylib` for
    // macOS, and both backends resolve it through that one home.  Only what gets
    // BUILT is host-specific.
    let os = loft::platform::host_lib_os();
    let libname = if os == loft::platform::LibOs::Macos {
        "libarity.dylib"
    } else {
        "libarity.so"
    };
    if os == loft::platform::LibOs::Windows {
        // TWO artifacts, because on Windows the two backends need different files
        // and Unix gets away with one.  `--interpret` LoadLibrary's the fixture at
        // run time, which only a DLL can satisfy; `--native` links it, and a DLL is
        // not linkable on its own — MSVC wants the import library beside it, named
        // exactly `<stem>.lib` because `add_c_library_flags` passes `-l arity`.
        // That is `platform::shim_implib_args`' rule, called here rather than
        // respelled, so the fixture cannot drift from what loft actually asks for.
        let dll = dir.join("arity.dll");
        let implib = dir.join("arity.lib");
        let cc = loft::platform::process::harness_command("cc")
            .args(["-O1", "-shared", "-o"])
            .arg(&dll)
            .arg(dir.join("arity.c"))
            .args(loft::platform::shim_implib_args(
                &implib.to_string_lossy(),
                os,
            ))
            .output()?;
        assert!(
            cc.status.success(),
            "the arity fixture must build: {}",
            String::from_utf8_lossy(&cc.stderr)
        );
        assert!(
            fa::exists(&implib),
            "`cc -shared` must also write the import library {} — without it the \
             --native link cannot resolve `-l arity`",
            implib.display()
        );
    } else {
        let cc = loft::platform::process::harness_command("cc")
            .args(["-O1", "-fPIC", "-shared", "-o"])
            .arg(dir.join(libname))
            .arg(dir.join("arity.c"))
            .output()?;
        assert!(
            cc.status.success(),
            "the arity fixture must build: {}",
            String::from_utf8_lossy(&cc.stderr)
        );
    }
    fa::write(
        dir.join("pkg/arity/loft.toml"),
        format!(
            "[library]\nname = \"arity\"\nversion = \"0.1.0\"\n\n[c]\nlibs = \"../../{libname}\"\n"
        ),
    )?;
    let sig = |n: usize| {
        let lp = (0..n)
            .map(|i| format!("p{i}: integer"))
            .collect::<Vec<_>>()
            .join(", ");
        let cs = vec!["int64_t"; n].join(", ");
        format!("pub fn ar{n}({lp}) -> integer;\n#c \"ar{n}\" \"int64_t({cs})\"\n")
    };
    // Sum of i^2 for i in 1..=n — hand-computable, and different for n and n+1.
    let want = |n: usize| (1..=n).map(|i| i * i).sum::<usize>();
    let run = |backend: &str, prog: &std::path::Path| -> std::io::Result<(String, String)> {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(dir.join("pkg"))
            .arg(prog)
            .output()?;
        Ok((
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
    };

    // Pass 1 — CALL SITES. The binding lives in a dependency; the program calls
    // it. At the ceiling both backends must call it and agree on the value; one
    // past it, both must refuse.
    for (n, expect_ok) in [(max, true), (max + 1, false)] {
        fa::write(src.join("arity.loft"), sig(n))?;
        let call = (1..=n).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let prog = dir.join(format!("call{n}.loft"));
        fa::write(
            &prog,
            format!("use arity::*;\nfn main() {{ println(\"R {{ar{n}({call})}}\") }}\n"),
        )?;
        let mut refused = Vec::new();
        for backend in ["--interpret", "--native"] {
            let (stdout, stderr) = run(backend, &prog)?;
            if expect_ok {
                assert!(
                    stdout.contains(&format!("R {}", want(n))),
                    "{backend} at the ceiling ({n} slots) must call and answer {}: \
                     stdout={stdout:?} stderr={stderr:?}",
                    want(n)
                );
            } else {
                assert!(
                    stderr.contains("c-binding-not-interpretable"),
                    "{backend} past the ceiling ({n} slots) must refuse: \
                     stdout={stdout:?} stderr={stderr:?}"
                );
            }
            refused.push(stderr.contains("c-binding-not-interpretable"));
        }
        // The point of the guard: not that each backend behaves, but that they
        // behave the SAME. The old bug passed a per-backend check.
        assert_eq!(
            refused[0], refused[1],
            "the two backends disagreed about {n} C slots"
        );
    }

    // Pass 2 — DECLARATION in code the author OWNS, never called. This is what
    // puts the error in front of the person who can fix it; before arc C the
    // only check was at the call site, so the author never saw it at all.
    let owned = dir.join("owned.loft");
    fa::write(
        &owned,
        format!(
            "{}fn main() {{ println(\"declared, never called\") }}\n",
            sig(max + 1)
        ),
    )?;
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg(&owned)
            .output()?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("c-binding-not-interpretable"),
            "{backend} must refuse an over-ceiling DECLARATION in owned code even \
             when nothing calls it: {stderr:?}"
        );
    }

    // ...but a DEPENDENCY declaring one the program never calls must still load:
    // a consumer cannot edit someone else's declaration, so it must not fail
    // their build. Mirrors how `superseded_fold_diagnostics` scopes itself.
    fa::write(
        src.join("arity.loft"),
        format!("{}{}", sig(max), sig(max + 1)),
    )?;
    let call = (1..=max)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let prog = dir.join("dep_ok.loft");
    fa::write(
        &prog,
        format!("use arity::*;\nfn main() {{ println(\"R {{ar{max}({call})}}\") }}\n"),
    )?;
    for backend in ["--interpret", "--native"] {
        let (stdout, stderr) = run(backend, &prog)?;
        assert!(
            stdout.contains(&format!("R {}", want(max))),
            "{backend}: a dependency's over-ceiling binding that is never called must \
             not break the build: stdout={stdout:?} stderr={stderr:?}"
        );
    }
    Ok(())
}

/// @PLN128 — the three shapes every numeric library (BLAS, LAPACK, FFTW, HDF5)
/// is actually made of, bound through `#c` and checked on both backends.
///
/// The load-bearing cell is `lc_daxpy`: **C writes its result THROUGH a
/// caller-supplied pointer**, which is how every BLAS and LAPACK routine
/// returns anything at all. If loft could not see those writes the numeric
/// stack would not be bindable, so this is the property the whole plan rests
/// on and it gets a guarantee probe rather than a one-off measurement.
///
/// The expected values are the ones `lc_selftest.c` computes in C (`make
/// check`), not numbers copied from a loft run — agreement between two loft
/// backends is not evidence that either matches C.
///
/// Skips when `cc` is absent, like its sibling above; a failed BUILD is a real
/// failure, not a skip.
/// @C107 @C109 — the C signature decides the count; a float return crosses `#c`.
#[test]
fn numeric_array_shapes_cross_identically_on_both_backends() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi");
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let built = loft::platform::process::harness_command("make")
        .arg("-C")
        .arg(&root)
        .output()?;
    assert!(
        built.status.success(),
        "the fixture library must build: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let prog = std::env::temp_dir().join("loft_pln128_numeric.loft");
    fa::write(
        &prog,
        "use lcabi::*;\n\
         fn main() {\n\
         \x20 a: vector<float> = [1.5, 2.25, 4.0];\n\
         \x20 println(\"dsum {lc_dsum_scaled(a)}\");\n\
         // C writes back through `y`. `y` is READ AFTERWARDS, which is also what\n\
         // keeps it alive across the call — a vector whose last use is the `#c`\n\
         // call itself is freed at that point and C's pointer dangles.\n\
         \x20 y: vector<float> = [1.0, 2.0, 3.0];\n\
         \x20 x: vector<float> = [10.0, 20.0, 30.0];\n\
         \x20 lc_daxpy(y, x, 1200);\n\
         \x20 println(\"daxpy {y[0]*1000.0} {y[1]*1000.0} {y[2]*1000.0}\");\n\
         // A Fortran scalar-by-reference against a C function that DOES take a\n\
         // count, so this one spends two slots. The Fortran cells below spend\n\
         // one, and that is the signature's decision rather than the type's.\n\
         \x20 s: vector<integer> = [6];\n\
         \x20 println(\"scalar {lc_scalar_ref(s)}\");\n\
         // The idiom the float refusal prescribes, on a COMPUTED double rather\n\
         // than a literal an author converted by hand.\n\
         \x20 v: vector<float> = [2.5];\n\
         \x20 out: vector<float> = [0.0];\n\
         \x20 lc_shim_scale(out, v);\n\
         \x20 println(\"shim {out[0]*1000.0}\");\n\
         \x20 fortran();\n\
         }\n\
         // @PLN128 arc D — the shape every real numeric library actually has:\n\
         // each argument a BARE pointer, no counts anywhere.  Before the count\n\
         // became the signature's decision, none of this was reachable — the\n\
         // honest declaration was refused for arity, and the shape loft insisted\n\
         // on delivered each count where the callee expected the next pointer,\n\
         // which SIGSEGV'd the interpreter and produced nothing under --native.\n\
         fn fortran() {\n\
         \x20 n: vector<integer> = [3];\n\
         \x20 al: vector<float> = [2.0];\n\
         \x20 be: vector<float> = [10.0];\n\
         \x20 x: vector<float> = [1.5, 2.25, 4.0];\n\
         \x20 y: vector<float> = [100.0, 200.0, 400.0];\n\
         \x20 lc_daxpby(n, al, x, be, y);\n\
         \x20 println(\"daxpby {y[0]*1000.0} {y[1]*1000.0} {y[2]*1000.0}\");\n\
         // `dgemm_` at full width: thirteen by-reference arguments, thirteen C\n\
         // slots.  This is the routine the ceiling was sized around.\n\
         \x20 d: vector<integer> = [2];\n\
         \x20 a2: vector<float> = [1.0, 2.0, 3.0, 4.0];\n\
         \x20 b2: vector<float> = [5.0, 6.0, 7.0, 8.0];\n\
         \x20 c2: vector<float> = [100.0, 200.0, 300.0, 400.0];\n\
         \x20 lc_dgemm(\"N\", \"N\", d, d, d, al, a2, d, b2, d, be, c2, d);\n\
         \x20 println(\"dgemm {c2[0]} {c2[1]} {c2[2]} {c2[3]}\");\n\
         // The two `char *` arguments have to land where the callee reads them.\n\
         // Asserting only the product above would pass with them misplaced,\n\
         // because the fixture computes the same product either way — it reports\n\
         // an unsupported transpose instead, so this cell is what pins them.\n\
         \x20 c3: vector<float> = [100.0, 200.0, 300.0, 400.0];\n\
         \x20 lc_dgemm(\"T\", \"N\", d, d, d, al, a2, d, b2, d, be, c3, d);\n\
         \x20 println(\"dgemm-t {c3[0]}\");\n\
         // Counted and bare in ONE signature, arranged so a left-to-right walk\n\
         // that grabs a count whenever an integer follows a pointer gives it to\n\
         // the wrong vector and answers a different number.\n\
         \x20 v2: vector<float> = [1.5];\n\
         \x20 w2: vector<float> = [2.0, 3.0];\n\
         \x20 println(\"split {lc_split(v2, 7, w2)}\");\n\
         \x20 elements();\n\
         }\n\
         // @PLN128 arc E — one cell per element WIDTH loft may hand over, each\n\
         // reader position-weighted in C so a stride that disagrees with loft's\n\
         // answers a different number rather than the right one by luck.  This\n\
         // is the half the declaration check lets through; the half it refuses\n\
         // is `a_vector_element_must_match_the_c_pointee`.\n\
         fn elements() {\n\
         \x20 a32: vector<u32> = [1000, 2000, 3000];\n\
         \x20 println(\"u32 {lc_u32_dot(a32)}\");\n\
         \x20 a16: vector<u16> = [10, 20, 30];\n\
         \x20 println(\"u16 {lc_u16_dot(a16)}\");\n\
         \x20 a8: vector<u8> = [1, 2, 200];\n\
         \x20 println(\"u8 {lc_u8_dot(a8)}\");\n\
         \x20 ac: vector<character> = ['A', 'B', 'C'];\n\
         \x20 println(\"char {lc_char_dot(ac)}\");\n\
         \x20 ab: vector<boolean> = [true, false, true];\n\
         \x20 println(\"bool {lc_bool_dot(ab)}\");\n\
         \x20 af: vector<single> = [0.5 as single, 1.25 as single, 2.5 as single];\n\
         \x20 println(\"single {lc_f32_dot(af)}\");\n\
         // The level-1 BLAS *function* shape: the answer comes back BY VALUE in\n\
         // an SSE register.  Refused until the caller grew a float-returning\n\
         // rung, which is what made `ddot_`/`dnrm2_`/`dasum_` need an ANSI-C\n\
         // shim each.  Both widths, because a C `float` return is a single in\n\
         // that register and reading those bits as a double is a denormal.\n\
         \x20 n3: vector<integer> = [3];\n\
         \x20 dx: vector<float> = [1.5, 2.5, 4.0];\n\
         \x20 dy: vector<float> = [4.0, 8.0, 16.0];\n\
         \x20 println(\"ddot {lc_ddot(n3, dx, dy)}\");\n\
         \x20 sx: vector<single> = [1.5 as single, 2.5 as single, 4.0 as single];\n\
         \x20 sy: vector<single> = [4.0 as single, 8.0 as single, 16.0 as single];\n\
         \x20 println(\"sdot {lc_sdot(n3, sx, sy)}\");\n\
         }\n",
    )?;
    let libdir = root.join("pkg");
    let run = |backend: &str| -> std::io::Result<String> {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    for want in [
        // 1.5 + 2.25 + 4.0 = 7.75, scaled by 1000.
        "dsum 7750",
        // a = 1.2; y[i] += a * x[i] over [1,2,3] and [10,20,30].
        "daxpy 13000 26000 39000",
        "scalar 42",
        "shim 5000",
        // Every value below is computed in C by `lc_selftest.c`, not read off a
        // loft run: agreement between two loft backends is not evidence that
        // either matches C.  y := 2*x + 10*y over [1.5, 2.25, 4] and
        // [100, 200, 400].
        "daxpby 1003000 2004500 4008000",
        // Column-major 2x2: 2*(A*B) + 10*C with A = [[1,3],[2,4]],
        // B = [[5,7],[6,8]], C = [[100,300],[200,400]].
        "dgemm 1046 2068 3062 4092",
        "dgemm-t -1",
        // 1.5*100 + 7*10 + (2*1 + 3*2), scaled by 1000.
        "split 228000",
        // @PLN128 arc E — one per element width, position-weighted so a wrong
        // stride cannot answer the right number.  All six are in
        // `lc_selftest.c` too, so C agrees with itself before loft is asked.
        "u32 14000",
        "u16 140",
        "u8 605",
        "char 398",
        "bool 4",
        "single 10500",
        // The float RETURN, both widths.  1.5*4 + 2.5*8 + 4*16 = 90.
        "ddot 90",
        "sdot 90",
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    let native = run("--native")?;
    assert_eq!(
        interp, native,
        "the two backends must agree on every numeric shape"
    );
    Ok(())
}

/// @PLN24 arc D — a C `char *` comes back as loft `text`, identically on both
/// backends, from libc alone (no fixture, no library to install).
///
/// `strerror` is the shape a real binding meets: borrowed storage the caller
/// must not free, a plain `int` argument, and an answer the C library owns. The
/// interpreter copies out of a raw return register and `--native` copies out of
/// a typed `*const c_void`; the two arrive at the same text or this crossing is
/// two mappings rather than one.
#[test]
fn a_c_string_return_crosses_identically_on_both_backends() -> std::io::Result<()> {
    let path = std::env::temp_dir().join("loft_pln24_c_textret.loft");
    fa::write(
        &path,
        "pub fn c_strerror(n: integer) -> text;   #c \"strerror\" \"char*(int)\"\n\
         pub fn c_strlen(s: text) -> integer;     #c \"strlen\" \"size_t(const char*)\"\n\
         fn main() {\n\
         \x20 e = c_strerror(2);\n\
         \x20 println(\"len {c_strlen(e)} chars {e.len()}\");\n\
         \x20 println(\"arg {c_strlen(c_strerror(2))}\");\n\
         }\n",
    )?;
    let mut seen = Vec::new();
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg(&path)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // The EXIT STATUS belongs in the message, because the interesting Windows
        // failure is a SILENT one: a binary that links but cannot find its DLL at
        // load time dies with `STATUS_DLL_NOT_FOUND` (0xC0000135) having written
        // nothing at all, so stdout and stderr are both empty and the assertion
        // said nothing without this.
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        // Not an exact string: `strerror(2)` is locale-dependent (it is "No such
        // file or directory" in the C locale). What IS pinned is that the text
        // survived the crossing — a non-empty answer whose byte length C agrees
        // with, which a truncated or NUL-terminated-at-zero copy fails.
        assert!(
            !stdout.contains("len 0 ") && !stdout.contains("arg 0"),
            "{backend}: an empty `strerror` means the crossing dropped the text:\n{stdout}"
        );
        seen.push(stdout);
    }
    assert_eq!(
        seen[0], seen[1],
        "the two backends must bring back the same text"
    );
    Ok(())
}

/// @PLN24 arc G — `c_library_available` is symbol-granular, not file-granular.
///
/// The cell that matters: a library that LOADS but does not export a symbol the
/// package declared. A file-granular answer says yes here and the call then
/// faults — the version-skew hole that makes a naive query worse than none, and
/// the reason the answer checks symbols at all.
///
/// The library is built here rather than mocked, and the control is the call
/// that WORKS (`sk_present(41)` is 42, by hand): without it a `false` could just
/// mean nothing loaded, which is the vacuous pass this test exists to refuse.
// @speed 1.1
#[test]
fn an_available_library_must_export_what_was_declared() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // the fixture library is built by `cc` here
    }
    let dir = std::env::temp_dir().join(format!("loft_skew_{}", std::process::id()));
    let pkg = dir.join("pkg/skewlib/src");
    fa::create_dir_all(&pkg)?;
    let src = dir.join("old.c");
    fa::write(&src, "long sk_present(long v) { return v + 1; }\n")?;
    let so = dir.join("libskew.so");
    let built = loft::platform::process::harness_command("cc")
        .args(["-O2", "-fPIC", "-shared", "-o"])
        .arg(&so)
        .arg(&src)
        .output()?;
    assert!(built.status.success(), "fixture library must build");

    // Forward slashes, because this path is about to be pasted into two string
    // literals — a TOML basic string and a loft one — and a Windows path is full
    // of escape sequences to both. `C:\Users\…` made the generated library fail
    // to lex at all: `error: Unknown escape sequence` at the `\U`. Escaping for
    // each syntax separately would work; using a separator neither treats as
    // special is simpler, and Windows accepts `/` everywhere loft passes this on
    // (`lib_variants` already splits a directory off on either separator).
    let so_str = so.to_string_lossy().replace('\\', "/");
    fa::write(
        dir.join("pkg/skewlib/loft.toml"),
        format!(
            "[library]\nname = \"skewlib\"\nversion = \"0.1.0\"\n\n[c]\noptional-libs = \"{so_str}\"\n"
        ),
    )?;
    fa::write(
        pkg.join("skewlib.loft"),
        format!(
            "pub fn sk_present(v: integer) -> integer;  #c \"sk_present\" \"long(long)\"\n\
             // Declared, and absent from this vintage of the library.\n\
             pub fn sk_newer(v: integer) -> integer;    #c \"sk_newer\" \"long(long)\"\n\
             pub const SKEW_SONAME = \"{so_str}\";\n\
             pub fn skew_ok() -> boolean {{ return c_library_available(SKEW_SONAME); }}\n"
        ),
    )?;
    let script = dir.join("probe.loft");
    fa::write(
        &script,
        "pub use skewlib::*;\nfn go() {\n  println(\"ok={skew_ok()}\");\n  println(\"call={sk_present(41)}\");\n}\ngo();\n",
    )?;

    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(dir.join("pkg"))
            .arg(&script)
            .output()?;
        let s = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend}: {s}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            s.contains("call=42"),
            "{backend}: the control must prove the library IS loaded and callable:\n{s}"
        );
        assert!(
            s.contains("ok=false"),
            "{backend}: a loadable library missing a declared symbol is NOT available:\n{s}"
        );
    }
    let _ = fa::remove_dir_all(&dir);
    Ok(())
}

/// @PLN23 — one uniform interface over four different C libraries.
///
/// `dump <D: SqlDb>` and `seed <D: SqlDb>` in the fixture never name a backend,
/// and every backend runs them unchanged. That is the whole claim, and it is not
/// a small one: sqlite STEPS a prepared statement, libpq MATERIALISES the result
/// and indexes it by (row, col), libmariadb streams rows as `char **`, and
/// duckdb materialises and is read by (col, row) with a caller-frees string.
/// Four result models behind one cursor.
///
/// **sqlite is the cell that keeps this honest.** It needs no server, so a
/// machine with no database still proves the interface, the bindings, the shim
/// loft compiled, and SQL NULL staying distinct from the empty string. Only
/// postgres and mariadb are conditional, and a skip is printed and recognised —
/// never silently counted as a pass, which is how a dead binding would otherwise
/// look green everywhere.
///
/// **duckdb is the fourth, and it is here because of @PLN24 arc G.** It was
/// proven once before and left out of the tree: no distro packages it and its
/// `.so` is 70 MB, so a REQUIRED declaration would have made every machine
/// running this test fetch it. Declared `[c] optional-libs` it costs nothing —
/// the fixture builds and runs without it, and says so — which is what makes
/// keeping a fourth backend in the tree cheap rather than a vendoring decision.
// @speed 2.1
#[test]
fn one_sql_interface_drives_four_different_c_libraries() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // the sqlite backend ships a shim loft must compile
    }
    // Which backends actually ran their assertions. A skip and a pass are the
    // same colour, so the count is checked at the end: this test used to gate
    // each backend on `libsqlite3.so.0` existing under `/lib/x86_64-linux-gnu`
    // | `/usr/lib` | `/usr/lib64`, and BOTH the spelling and the directories are
    // Linux's. On macOS `/usr/lib` exists but holds `libsqlite3.dylib`, so every
    // conditional cell — including sqlite, written to be the unconditional one —
    // skipped in silence and the test passed on macOS for months having opened
    // no database. The availability question now has ONE home, loft's own
    // `c_library_available`, which translates the declared soname to the host's
    // spelling; a mode that cannot run says `SKIP` and is counted here.
    let mut ran: Vec<&str> = Vec::new();
    // No availability question is asked from out here. Every backend answers it
    // with `c_library_available` — true only when the library opens AND every
    // declared symbol resolves — and prints `SKIP`. A second question asked from
    // the harness could only be the weaker, file-granular one, and two answers
    // to one question is how an unusable library got called (loft#770).
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("uniform.loft");
    let run = |backend: &str, mode: &str| -> std::io::Result<String> {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .env("LOFT_SQLDB_MODE", mode)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // Exit status in the message, for the same reason as the sites above: a
        // Windows binary that cannot find a DLL dies before `main` with both
        // streams empty, and without the code the message says nothing at all.
        assert!(
            out.status.success(),
            "{backend}/{mode} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };

    // The three rows every backend is given, rendered by the SAME generic code:
    // a value, SQL NULL, and the empty string.
    let expect = "[ada] <null> [] ";

    // @PLN23 S4 — the prepared-statement line, and every token in it is a
    // separate claim:
    //
    //   p=2      two holes, and the DRIVER's own parameter count agreed with the
    //            parser's (each backend fails `prepare` outright when they differ)
    //   [ada] <null> []   value / SQL NULL / empty string, still three answers
    //                     — now arriving through the BIND path rather than as
    //                     literals in the statement text
    //   ['); DROP TABLE loft_p; --]
    //            the cell that matters. Spliced into SQL this closes the VALUES
    //            list and drops the table; bound, it is stored verbatim. That the
    //            SELECT after it returns rows at all is the proof the table
    //            survived, so this assertion cannot pass vacuously.
    //   hit=4    the same hostile text found its own row by EQUALITY, which it
    //            could only do by crossing intact as data rather than as syntax.
    //   big=1000 a value far past the 256-byte column buffer mariadb's result
    //            binds start with, round-tripped at full length — the truncation
    //            re-fetch is exercised, not merely written.
    //
    // @PLN23 H6 — the identifier, the ONE hole that is not a value. Four tokens,
    // two of which move in opposite directions if the type stops carrying it:
    //
    //   ident=4  a `SqlIdent` named the TABLE the query reads, so it reached the
    //            statement as SYNTAX. No placeholder stands for a table name, so
    //            a `SqlIdent` that went down the bind path instead would not have
    //            found the wrong row — it would have failed to prepare at all.
    //            The quoting is per-dialect and this is where that is measured:
    //            giving mariadb the ANSI quote the other three use answers
    //            `FAIL q8 … error in your SQL syntax … near '"loft_p" WHERE`,
    //            because it reads a double-quoted name as a string literal. The
    //            backtick is not a preference, and choosing the quote at ASSEMBLY
    //            time rather than when the hole was filled is what allows it.
    //   refused=true   the same construction given `loft_p; DROP TABLE loft_p; --`
    //            is refused AT CONSTRUCTION — before any statement was built
    //            around it, and long before a server could be asked about it.
    //   ran=false      the statement built on the refused identifier has no text.
    //            `statement` answers `text?` and every backend must discharge it,
    //            so nothing is sent.
    //   alive=true     and the table is still there. Vacuous only if `dump`
    //            itself were broken, which the `[ada] <null> []` cells rule out.
    let bound = "p=2 [ada] <null> [] ['); DROP TABLE loft_p; --] hit=4 big=1000 \
                 ident=4 refused=true ran=false alive=true";

    // @PLN23 T1–T3 — the transaction line, from one generic `transact<D: SqlDb>` that
    // names no backend. It lands before the object mapping because writing a collection
    // non-atomically is not a smaller step, it is a wrong one.
    //
    //   nested=false      a `db_begin` INSIDE a transaction is REFUSED. The silent
    //                     no-op is the dangerous version: the inner "rollback" would
    //                     discard the OUTER transaction's work, and nothing afterwards
    //                     can tell.
    //   rows=0/1          rollback DISCARDED the row, commit KEPT it. The pair is what
    //                     makes the cell non-vacuous — three of these four fields move
    //                     when the transaction is a fiction.
    //   stray=false/false commit or rollback with nothing open is refused.
    //
    // Proven to FIRE by replacing sqlite's three methods with `return true` (a backend
    // that reports transactions it never opens): `nested=true rows=1/2 stray=true/true`.
    let tx = "begin=true/true nested=false rollback=true commit=true rows=0/1 stray=false/false";

    // @PLN23 H7 — procedures, from one generic `procedures<D: SqlDb>` that names no
    // backend. Two of the four put the definition in the SERVER's catalogue
    // (`CREATE OR REPLACE PROCEDURE`, then `CALL`) and two keep it in the PROCESS
    // and prepare it on call; this line is what says a caller cannot tell which.
    //
    //   deployed=true  the definition was accepted. The body's parameters are typed
    //                  by the holes' own loft types, so a procedure is declared by
    //                  writing its statement rather than declaring a signature twice.
    //   called=true    running it did the work, and `rows=1` is that work seen from
    //                  OUTSIDE the procedure — the pair is what makes the cell
    //                  non-vacuous. Proven to fire by replacing sqlite's `db_call`
    //                  with `return true`: `guard=true rows=0`, two fields moving.
    //   guard=false    the same procedure called with the wrong NUMBER of values is
    //                  refused before anything is sent.
    //   ctl=false      a two-statement body is refused AT DEPLOY, everywhere. sqlite
    //                  and duckdb have no procedural language; mariadb and postgres
    //                  each have one and they are not the same one (SQL/PSM vs
    //                  plpgsql), so there is no such body a uniform API could carry.
    //                  Proven to fire by making `procedural` never refuse: `ctl=true`.
    let proc = "deployed=true called=true guard=false rows=1 ctl=false";

    // sqlite — no server, so whenever the library is here a failure is real.
    //
    // The availability question is the PROGRAM's to answer, exactly as it is for
    // postgres and maria below. Asking it from out here needs a second, WEAKER
    // question — "does a file with this name load" — and that one says yes for
    // an unrelated library sharing the translated name, which is how a hostile
    // `sqlite3.dll` on PATH turned a skip into an access violation on Windows
    // (loft#770). `c_library_available`, which the backend uses, is true only
    // when every declared symbol resolves. One home for the fact; the harness
    // reads the verdict off stdout.
    let s = run("--interpret", "sqlite")?;
    if s.contains("SKIP") {
        assert!(
            s.contains("not installed"),
            "an absent library must be REPORTED, not inferred from silence:\n{s}"
        );
    } else {
        ran.push("sqlite");
        assert!(
            s.contains(&format!("sqlite {expect}")),
            "sqlite must render value / NULL / empty distinctly:\n{s}"
        );
        assert!(
            s.contains(&format!("sqlite bound {bound}")),
            "sqlite: a bound value must reach the server as DATA, never as syntax:\n{s}"
        );
        assert!(
            s.contains(&format!("sqlite tx {tx}")),
            "sqlite: rollback must discard, commit must persist, and a nested begin \
             must be REFUSED:\n{s}"
        );
        assert!(
            s.contains(&format!("sqlite proc {proc}")),
            "sqlite: a procedure kept in the PROCESS must deploy, call and do the \
             work, and refuse a body needing a procedural language:\n{s}"
        );
        // @PLN133 P3 — a bound float must come back BIT-IDENTICAL.
        //
        // **sqlite is pinned at 6 of 7, which is its real answer.**  It sends a
        // float as TEXT (no `#c` path carries a `double` by value) and its own
        // text→REAL converter rounds `-5.196972490273514e-183` one ULP wrong,
        // where `sqlite3_bind_double` on the same value is right — measured
        // directly.  The fix needs `#c` float support (@PLN128 E3): it cannot go
        // in the shim, which is deliberately free of sqlite symbols so it links
        // where the optional library is absent (@PLN24 arc G).  The other three
        // parse correctly and are held to 7/7 below.
        //
        // This cell read 7/7 for a while, and that was an ARTEFACT rather than a
        // pass: `floats` is itself generic, so under loft#791 its write loop and
        // its read loop saw the same corrupted vector and agreed with each other.
        // Fixing #791 made the guard honest and the real defect reappeared.  Keep
        // the value that exposes it; when E3 lands, raise this to 7/7.
        //
        // The per-backend READ expression is still load-bearing for a different
        // reason: sqlite renders a `REAL` as `%!.15g`, so reading the column
        // naively loses the low bits of values that ARE stored correctly.
        //
        // The one-ULP miss is the LIBRARY's, so it is a fact per sqlite version:
        // 3.46.1 (measured 2026-09-13) parses that text exactly and answers 7/7,
        // the older library 6/7.  Both are the honest answer for their sqlite;
        // any other count is loft's defect.
        assert!(
            s.contains("sqlite float wrote=7 exact=6/7 inlined=false plain=true")
                || s.contains("sqlite float wrote=7 exact=7/7 inlined=false plain=true"),
            "sqlite: a bound float must round-trip exactly (see @PLN133 P3):\n{s}"
        );
        assert_eq!(
            run("--native", "sqlite")?,
            s,
            "both backends, one interface"
        );
    }

    // postgres and mariadb — conditional, and a skip is recognised as a skip.
    // The condition is the library's own answer plus a reachable server; both
    // arrive as `SKIP` on stdout rather than being guessed at from out here.
    for mode in ["postgres", "maria"] {
        let out = run("--interpret", mode)?;
        if out.contains("SKIP") {
            continue; // library absent, or no server reachable here
        }
        ran.push(mode);
        assert!(
            out.contains(&format!("{mode} {expect}")),
            "{mode} must render the same three cells as sqlite:\n{out}"
        );
        // Byte-identical to sqlite's, from the same generic `bound<D: SqlDb>`:
        // three placeholder dialects, three bind APIs, three result models, one
        // answer. A backend that quietly concatenated would differ HERE and
        // nowhere else, which is what makes the line worth comparing whole.
        assert!(
            out.contains(&format!("{mode} bound {bound}")),
            "{mode}: the bound statement must give the same answer as sqlite:\n{out}"
        );
        // The same generic `transact<D: SqlDb>`, byte-identical to sqlite's answer:
        // three servers with three transaction implementations, one contract.
        assert!(
            out.contains(&format!("{mode} tx {tx}")),
            "{mode}: transactions must behave exactly as sqlite's do:\n{out}"
        );
        // The interesting half of H7: this backend put the definition in its own
        // catalogue and CALLed it, sqlite kept it in the process — and the line is
        // byte-identical. Where a procedure lives is not something a caller can see.
        assert!(
            out.contains(&format!("{mode} proc {proc}")),
            "{mode}: a server-side procedure must give the same answer as the \
             process-side emulation:\n{out}"
        );
        // @PLN133 P3 — a bound float must come back BIT-IDENTICAL.
        //
        // Every backend sends a float as TEXT (no `#c` path carries a `double`
        // by value), so each is relying on its server's text→double conversion
        // being correctly rounded.  Three of the four are.  **sqlite is not**:
        // it loses the last bit of `-5.196972490273514e-183`, measured directly
        // against `sqlite3_bind_double`, which gets it right.  Fixing it needs
        // `#c` float support (@PLN128 E3) — it cannot go in the shim, which is
        // deliberately free of sqlite symbols so it links where the optional
        // library is absent (@PLN24 arc G).
        //
        // Both of these servers parse correctly, so they are held to 7/7 —
        // sqlite is the odd one out and is pinned separately, above.
        assert!(
            out.contains(&format!(
                "{mode} float wrote=7 exact=7/7 inlined=false plain=true"
            )),
            "{mode}: a bound float must round-trip exactly (see @PLN133 P3):\n{out}"
        );
        assert_eq!(
            run("--native", mode)?,
            out,
            "{mode}: both backends must agree"
        );
    }

    // @PLN24 arc G — duckdb, declared `[c] optional-libs`.
    //
    // This cell is unconditional ON PURPOSE, and it is the one that proves the
    // arc: every mode above needs its C library installed to run at all, and
    // before arc G a declared-but-absent library failed the `--native` LINK, so
    // a program that never called into it did not build. Here the program is
    // expected to build and run either way — the only question is which of the
    // two answers it gives.
    let out = run("--interpret", "duckdb")?;
    let native = run("--native", "duckdb")?;
    assert_eq!(
        native, out,
        "duckdb: both backends must agree about an optional library"
    );
    if out.contains("SKIP") {
        // libduckdb absent — the interesting half. The program still ran.
        assert!(
            out.contains("not installed"),
            "an absent optional library must be REPORTED, not inferred from silence:\n{out}"
        );
    } else {
        // libduckdb present — then it is held to exactly the bar the other three meet,
        // and the arc-G property costs it no leniency. Proven on 1.5.5 with the library
        // reachable via `LD_LIBRARY_PATH`; no system install is needed, and none is
        // assumed here, which is why this stays conditional.
        ran.push("duckdb");
        assert!(
            out.contains(&format!("duckdb {expect}")),
            "duckdb must render the same three cells as sqlite:\n{out}"
        );
        assert!(
            out.contains(&format!("duckdb bound {bound}")),
            "duckdb: a bound value must reach the server as DATA, never as syntax:\n{out}"
        );
        assert!(
            out.contains(&format!("duckdb tx {tx}")),
            "duckdb: rollback must discard, commit must persist, and a nested begin \
             must be REFUSED — `BEGIN TRANSACTION` is its own spelling:\n{out}"
        );
        assert!(
            out.contains(&format!("duckdb proc {proc}")),
            "duckdb: the process-side procedure registry is SHARED with sqlite's, so \
             its answer must be sqlite's:\n{out}"
        );
    }

    // The guard that would have caught the Linux-shaped probe.
    //
    // Everything above is conditional, so with no cell running this test is a
    // green that asserted nothing — which is exactly what it was on macOS.
    // Naming the backends that ran turns a silent evaporation into a readable
    // one, and requiring sqlite on Linux pins the reference platform: it is the
    // cell with no server to be unreachable, so there it can only be missing if
    // something upstream broke.
    println!("@PLN23 backends exercised: {ran:?}");
    // approved exemption: a system sqlite the CI image guarantees (`LOFT_REQUIRE_SQLITE=1`) exists on Linux only; Windows substitute: the cells run where `winsqlite3` loads, and `ran` is printed
    if cfg!(target_os = "linux") {
        assert!(
            ran.contains(&"sqlite"),
            "sqlite must run on Linux — it needs no server, so a skip here means \
             the library went missing or the availability question broke, and a \
             pass with it skipped asserts nothing. Exercised: {ran:?}"
        );
    }
    Ok(())
}

/// @PLN138 — two cursors from one connection, and the resource each one owns.
///
/// The claim this whole plan exists to make. @PLN23 kept the cursor as state ON
/// the connection because loft interfaces had no associated types, so a contract
/// could not say "and this connection yields a cursor of ITS OWN kind"; the price
/// was that a connection held exactly one, and a second `db_select` silently
/// replaced the first. `type Rows: SqlRows` states it, and the fixture measures
/// it — every function in it is generic over `SqlDb` and names no backend.
///
/// **UNCONDITIONAL, like the pure-derivation cell above and for the same reason.**
/// sqlite needs no server, so a machine with no database still proves the
/// guarantee. Every other SQL test here skips where a library or a server is
/// missing; a guarantee whose gate can evaporate into a green is not one.
///
/// Two claims, and the second is the one a fixture alone cannot make:
///
///   - the VALUES, hand-computed per cell inside the script. Row 2's name is SQL
///     NULL and row 3's is the empty string, so the interleaved pairing shows
///     each cursor held its own position rather than reading the other's row.
///   - the RELEASE, answered by sqlite itself. Forty cursors are abandoned
///     mid-walk and the connection is then closed; `sqlite3_close` reports
///     `SQLITE_BUSY` while any statement on it is unfinalized, so a scope end
///     that failed to release is a return code rather than a leak inferred from
///     memory growth. Verified to FAIL with the hook removed.
///
/// The script counts its own failures and withholds the summary line when there
/// are any — a `check` that only printed would let a run with failures in it
/// still print `two cursors ok`, which is exactly the false green this file's
/// other tests are written against.
#[test]
fn one_connection_yields_two_independent_cursors() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // the sqlite backend ships a shim loft must compile
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("two_cursors.loft");
    let mut ran = false;
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            continue;
        }
        ran = true;
        assert!(
            stdout.contains("two cursors ok") && !stdout.contains("FAIL"),
            "{backend}: a connection must yield independent cursors:\n{stdout}"
        );
        // A wrong free is refused rather than performed, so it costs a printed
        // line and not a wrong answer — which means the values above can all
        // agree while the ownership underneath is broken.
        assert!(
            !stdout.contains("BUG (#"),
            "{backend}: cursors must not provoke an internal fault:\n{stdout}"
        );
    }
    assert!(
        ran,
        "sqlite needs no server, so a skip here means the library went missing \
         or the availability question broke — and a pass with it skipped asserts \
         nothing about the guarantee"
    );
    Ok(())
}

/// @PLN133 S2–S5 — one table definition, derived and reconciled, with no
/// database anywhere.
///
/// Drop the sqldb fixtures' `note:` advisories before comparing two backends' stdout.
///
/// Each of those fixtures deletes its temp database on the way out and prints
/// `note: … was not removable` when the file survives.  That reports on the ENVIRONMENT,
/// not on the answer the oracle is about.  On Windows a database file's handle can outlive
/// the process that held it for a moment, so the first backend's run reports the note and
/// the second — run straight after, against the same path — does not; the two stdouts then
/// differ by a line neither backend computed, and a green leg went red at a commit that had
/// changed nothing about either path (loft#834).
///
/// Only the advisory is dropped.  Every line carrying a value, a count, or a driver hit is
/// still compared, and each backend is still checked against `expect` with the note in
/// place — so a fixture that genuinely stopped cleaning up on both backends is still
/// visible, and the failure this filters is the one that is not a difference in behaviour.
fn answer_without_housekeeping(stdout: &str) -> String {
    stdout
        .lines()
        .filter(|l| !l.trim_start().starts_with("note: "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The whole point of `TableDef` being a VALUE is that it is built two ways —
/// derived from a loft type, or read back from a database — and consumed four,
/// with nothing downstream allowed to ask which. That makes the derivation
/// testable with no connection at all: the hand-built `have` definitions in the
/// script stand in for what `introspect` will read, and they are the same shape,
/// which is the invariant rather than a shortcut.
///
/// **So this cell is UNCONDITIONAL**, and that is what it is for. Every other
/// SQL test in this file skips where a library or a server is missing; this one
/// runs on any machine, so the derivation the reader and the writer must agree
/// on has a gate that cannot evaporate into a green.
///
/// The DDL it asserts is HAND-WRITTEN per dialect. Comparing one generator
/// against another proves they agree, which is not the question.
#[test]
fn one_table_definition_derives_reconciles_and_renders() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        // The `schema` package holds no `#c` at all, but it sits beside the
        // backends in one lib directory and `sql` is compiled with it.
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("schema_pure.loft");
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success() && stdout.contains("schema ok"),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        // A leak here is not cosmetic. This derivation runs on the LAZY FETCH
        // path, once per miss, so a record left behind per call is a traversal
        // that grows the heap for as long as it runs.
        assert!(
            !stdout.contains("not freed"),
            "{backend}: the derivation must leave nothing behind:\n{stdout}"
        );
    }
    Ok(())
}

/// @PLN133 S6 — the definition loft WROTE, read back out of sqlite's catalogue.
///
/// The pure gate above proves the derivation against hand-built definitions.
/// This proves the hand-built ones were the right shape: `derive` from a loft
/// type and `introspect` from the database produce one value, and `reconcile`
/// matches them. That round trip is what requirement 2 rests on — a writer and a
/// reader deriving their SQL separately agree only until they do not, and
/// nothing else checks it.
///
/// **Run twice, and the second run is the one that matters.** Into an EMPTY
/// database, where loft creates the schema; and against a table made by hand
/// with a scrambled column order, a float kept in a `VARCHAR`, a boolean kept in
/// `TEXT` and one extra column, where loft follows it. The first run passes even
/// if `reconcile` always agrees.
#[test]
fn a_table_loft_wrote_and_a_table_loft_found_are_one_value() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // the sqlite backend ships a shim loft must compile
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("schema_live.loft");

    // Every token is a separate claim, and three of them move in different
    // directions if the round trip is a fiction:
    //
    //   created cols=4 ix=1 …   loft's own DDL came back as four columns AND the
    //           index its collection kind implies. @PLN129 refuses a bind whose
    //           lookup no index serves, so a writer that omitted it would build
    //           a database its own reader cannot open.
    //   declared flag=INTEGER   sqlite has no boolean type, so loft wrote one as
    //           INTEGER — and the BINDING is what turns it back into a boolean.
    //           That is why `flag conversion=ConvBoolean` is beside it: the pair
    //           is the claim, and either alone would look right while wrong.
    //   followed cols=5 bound=true   a table loft did not write, with the columns
    //           in a different ORDER and an extra one, is readable. Column order
    //           in a foreign table means nothing and must never be read as
    //           meaning.
    //   varchar score conversion=ConvFloat   a float kept in a VARCHAR is the
    //           same conversion as one kept in a REAL, because every driver hands
    //           the value over as text.
    //   noindex bound=false     …and the refusal NAMES the column, because a
    //           refusal a DBA cannot act on is only a slower failure.
    //   extra bound=true write=false   ONE table, two verdicts. An unknown NOT
    //           NULL column with no default is perfectly readable and impossible
    //           to INSERT into.
    //
    // @PLN23 S7c adds a migration that RUNS, and the rows are the gate:
    //
    //   grown rows 1|ada|null;2|grace|null;   the two original rows, untouched,
    //           and the added column reading NULL rather than a fabricated
    //           value. A plan changes a table's SHAPE and never its CONTENT, and
    //           this line is where that stops being a sentence.
    //   grown before bound=false … / grown after bound=true write=true   the
    //           binding that refused now works, and the plan came off the SAME
    //           comparison that refused — migration and binding disagree about
    //           nothing.
    //   moved rows 7|seven;8|eight;   a DECLARED rename, and the values
    //           travelled with the name. Undeclared it would be an ADD plus an
    //           orphan, which is what "indistinguishable from a drop plus an
    //           add" means.
    let expect = [
        "created cols=4 ix=1 bound=true write=true why=",
        "declared flag=INTEGER score=REAL",
        "flag conversion=ConvBoolean",
        "followed cols=5 bound=true write=true why=",
        "varchar score conversion=ConvFloat",
        "noindex bound=false why=table noix_person has no index on id",
        "extra bound=true write=false why=column tenant is NOT NULL with no default",
        // @PLN23 S7c — the migration ran, and the CONTENT is the claim. A plan
        // that changed content would show up as a value that moved.
        "grown before bound=false why=table grown has no column memo",
        "grown plan ready=true steps=1",
        "grown after bound=true write=true why=",
        "grown rows 1|ada|null;2|grace|null;",
        "moved plan ready=true steps=1",
        "moved rows 7|seven;8|eight;",
        "schema_live ok",
    ];

    let mut first: Option<String> = None;
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            // The availability question is the PROGRAM's, answered by
            // `c_library_available` — true only when every declared symbol
            // resolves. A skip is printed, never inferred from silence.
            assert!(
                stdout.contains("not installed"),
                "an absent library must be REPORTED:\n{stdout}"
            );
            return Ok(());
        }
        for line in expect {
            assert!(
                stdout.contains(line),
                "{backend}: expected `{line}` in:\n{stdout}"
            );
        }
        // Both backends, one derivation. A backend that derived its own would
        // differ HERE and nowhere else, which is what makes the whole output
        // worth comparing rather than each line.
        match &first {
            None => first = Some(stdout),
            Some(f) => assert_eq!(
                answer_without_housekeeping(f),
                answer_without_housekeeping(&stdout),
                "both backends, one table definition"
            ),
        }
    }
    Ok(())
}

/// @PLN23 S6 — the child tables a collection field implies, against a real
/// engine.
///
/// The pure gate proves the DERIVATION; this proves it was a schema an engine
/// accepts and a round trip that closes. That distinction is the plan's own
/// history: the cleanest version of the addressing rule — *the declared key
/// addresses an element* — was falsified by an `INSERT`, not by re-reading the
/// design (OBJECT_MAPPING.md § What the probe falsified).
///
/// Three lines, and each moves in a different direction if the address rule is
/// wrong:
///
///   docs 7|seven;9|nine;11|eleven   doc 11's tag vector is EMPTY and doc 9's
///           score vector is. A parent with no children is still a row, and a
///           write path that emitted a parent only when it had children would
///           lose it silently.
///   scores 7|0|10;7|1|20;11|0|30   the S6a shape, unchanged by S6b landing
///           beside it. Two collections of different kinds under one owner, and
///           the field path in the table NAME is what keeps them apart.
///   tags 7|0|a|1|~;7|1|b|2|;9|0|a|1|x;9|1|a|1|x;9|2|c|3|~   the S6b shape, and
///           three claims at once. Doc 9 holds the SAME tag twice: under a
///           key-addressed rule those collapse to one row, and under the ordinal
///           rule they are rows 0 and 1 — the falsified claim, standing as a
///           test. `a/1` is also doc 7's, so an owner column that went missing
///           MERGES two documents rather than losing anything. And `~` is SQL
///           NULL beside `b`'s empty string: not the same value, which is most
///           of why a binding exists.
///
/// @PLN23 S6c adds the two KEYED shapes, and the kind is the only thing that
/// decides between them:
///
///   seen ord=false ix=1 rank ord=true ix=2   a `hash` addresses by its declared
///           key, so it carries no ordinal and one index; a `sorted` takes the
///           ordinal and owes its key an index of its own. @PLN129 refuses a
///           bind whose lookup no index serves, so the second one is not
///           decoration — and a `sorted` sub-collection had neither it nor a
///           refusal before S6c.
///   seen 7|m1|10;7|m2|20;9|m1|99   `m1` under two owners. A hash key is unique
///           WITHIN its collection, and the owner column is the only thing
///           keeping the two apart.
///   rank 7|0|1|a;7|1|2|b;7|2|3|c;…   the steps went in as 3,1,2. The ordinal is
///           the COLLECTION's order, not the order a program added things in,
///           and only an out-of-order insertion can tell those apart.
///   beats 1|0|1|x;1|1|2|z;1|2|3|y   added as bars 1, 3, 2: the ordinal is the
///           collection's order again, here for a `sorted` whose element type has
///           a second keyed view (so it is stored by reference).  Every bar is
///           distinct — a `sorted` keeps one record per key (loft#1572).
///   byname 1|1|x;1|3|y;1|2|z   the SAME three records, addressed by the other
///           view's key. Only `beats` was written to; two keyed collections over
///           one element type are views of one record set (loft#843), so both
///           child tables hold all three.
///
/// @PLN23 S7 recurses: a collection inside a record ELEMENT is a table of its
/// own, addressed by (the root's key, the parent element's address, its own).
///
///   pieces 1|0|0|10|1|2;1|0|1|11|3|4;…;2|1|0|14|9|0
///           Two ledgers, two marks each, two pieces each — the smallest shape
///           where each of the THREE address levels is separately load-bearing.
///           Both ledgers have a mark `A` and both `A`s hold pieces 10 and 11,
///           so dropping `ledger_id` merges the ledgers, dropping `ord` merges
///           `A`'s pieces with `B`'s, and dropping `ord_2` collapses the pieces
///           within one mark. Three different wrong answers, and none of them is
///           "fewer rows than expected".
///   marks 1|0|A;1|1|B;2|0|A;2|1|C   the repeated label, one level up.
///   …|10|1|2   the last two columns are `at_row` / `at_col`: an INLINE struct
///           inside the grandchild's element, flattened (@PLN23 S7b). It has no
///           identity, so it has no table — and the write reads it through the
///           column's PATH, which is what `field_value(x, [outer, inner])` was
///           added to loft for.
///   notes 1|0|100;1|1|200;2|0|300   @PLN23 S7b-ii — a collection inside an
///           INLINE struct. `(ledger_id, ord)` and no more: `meta` has no
///           identity, so it adds to the table NAME and to nothing else, while
///           its sibling scalar `rev` flattens into the owner's own row
///           (`ledger 1|one|5`). A record ELEMENT in the same position DOES add
///           an ordinal — that is the distinction, and it is the mapping's one
///           rule (identity earns an address) applied one level in.
///   permute at_col=4,at_row=3,pid=11,ord_2=1,ord=0,ledger_id=1,   ONE row built from a REVERSED
///           definition. The address columns come out outer→inner, so the two
///           ordinals are always in depth order and a writer counting them in
///           ROW order agrees on every other line here. Reversing the columns is
///           the only thing that tells `ords[c.depth]` from a counter — without
///           it, `ColumnDef.depth` is a field no test could distinguish from one.
///
/// sqlite, so a machine with no database still runs it.
#[test]
fn a_collection_field_becomes_child_rows_a_real_engine_gives_back() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // the sqlite backend ships a shim loft must compile
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("children_live.loft");

    let expect = [
        "tables=5 parent=doc scores=doc_scores tags=doc_tags",
        "seen ord=false ix=1 rank ord=true ix=2",
        "docs   7|seven;9|nine;11|eleven",
        "scores 7|0|10;7|1|20;11|0|30",
        "tags   7|0|a|1|~;7|1|b|2|;9|0|a|1|x;9|1|a|1|x;9|2|c|3|~",
        "seen   7|m1|10;7|m2|20;9|m1|99",
        "rank   7|0|1|a;7|1|2|b;7|2|3|c;9|0|6|g;9|1|8|h",
        "beats  1|0|1|x;1|1|2|z;1|2|3|y",
        "byname 1|1|x;1|3|y;1|2|z",
        "depth  tables=4 grandchild=ledger_marks_pieces inline=ledger_meta_notes",
        "ledger 1|one|5;2|two|6",
        "marks  1|0|A;1|1|B;2|0|A;2|1|C",
        "pieces 1|0|0|10|1|2;1|0|1|11|3|4;1|1|0|12|5|6;1|1|1|13|7|8;2|0|0|10|1|2;2|0|1|11|3|4;2|1|0|14|9|0",
        "notes  1|0|100;1|1|200;2|0|300",
        "permute at_col=4,at_row=3,pid=11,ord_2=1,ord=0,ledger_id=1,",
        "children_live ok",
    ];

    let mut first: Option<String> = None;
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            return Ok(());
        }
        for line in expect {
            assert!(
                stdout.contains(line),
                "{backend}: expected `{line}` in:\n{stdout}"
            );
        }
        match &first {
            None => first = Some(stdout),
            Some(f) => assert_eq!(
                answer_without_housekeeping(f),
                answer_without_housekeeping(&stdout),
                "both backends, one set of child tables"
            ),
        }
    }
    Ok(())
}

/// @PLN133 S7 — one connection string, and the registry that turns it into a
/// connection every consumer can use.
///
/// `SqlDb` is satisfied by four unrelated types and loft interfaces are STATIC
/// dispatch, so no function can return "one of them". The registry is a
/// struct-enum that satisfies the interface itself — and the strongest claim in
/// the fixture is one nothing asserts: `uniform` there is generic over `SqlDb`
/// and is handed an `AnyDb`. If the enum did not satisfy the interface the
/// program would not compile, which is the entire question S7 had to answer.
///
/// **Unconditional**, like the pure schema gate beside it: this half opens no
/// library, so it cannot skip into a green that asserted nothing. The cells that
/// need a database live in the live gate below.
#[test]
fn one_connection_string_reaches_its_driver_and_a_refusal_behaves_like_one() -> std::io::Result<()>
{
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        // The registry holds no `#c` itself, but it names all four backends in
        // one type, so their shims are compiled with it.
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("registry_pure.loft");
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success() && stdout.contains("registry ok"),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !stdout.contains("not freed"),
            "{backend}: a refused connection must leave nothing behind:\n{stdout}"
        );
    }
    Ok(())
}

/// @PLN133 S7 — the schema round trip, over a connection ONE STRING opened.
///
/// The pure gate above proves a string reaches its driver. This proves the
/// connection it produced is a connection: `introspect` — generic over `SqlDb`
/// and written before the registry existed — takes the enum unchanged, a cursor
/// walks rows through it, and a transaction lands on the same connection the
/// insert did.
///
/// **The cursor cell is the one that could quietly fail.** A variant holds a
/// COPY of the backend struct, so the handle `db_select` writes and `db_next`
/// reads has to live INSIDE the enum. A copy that came apart reads as an empty
/// result set — which looks exactly like an empty table.
#[test]
fn a_connection_the_registry_opened_is_a_connection() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("registry_live.loft");

    // Each token is a separate claim, and they fail in different directions:
    //
    //   connected backend=sqlite quote="   the STRING chose the driver, and the
    //           dialect came with it rather than being asked for separately.
    //   round trip cols=4 ix=1 bound=true   `derive` → `render` → `introspect` →
    //           `reconcile`, all over the enum. @PLN129 refuses a bind whose
    //           lookup no index serves, so `ix=1` is what makes the table usable.
    //   cursor names=ada grace alan   bound INSERTs and a walked cursor, so the
    //           mutation reached inside the variant. An enum holding a stale copy
    //           answers "" here, which is what an empty table also answers.
    //   float naive=… dialect=true   `dialect=true` is the claim: the driver's
    //           read gives back the full-mantissa double it wrote.  `naive` is what the
    //           portable `SELECT score` answers, and that is the SQLite library's
    //           choice: one that renders a REAL as %!.15g loses the low bits
    //           (`naive=false`, which shows the value was hard), one that renders it
    //           round-trip exact (3.53 does) reads it whole (`naive=true`).  Either is
    //           accepted; what may never happen is `dialect=false`.
    //   tx … rows=3/4   rollback discarded, commit kept. A db_begin that quietly
    //           did nothing answers 4/4 and one that discarded everything 3/3.
    let expect = [
        "connected backend=sqlite quote=\"",
        "round trip cols=4 ix=1 bound=true",
        "cursor names=ada grace alan",
        "dialect=true\n",
        "tx begin=true/true rollback=true commit=true rows=3/4",
        "registry_live ok",
    ];

    let mut first: Option<String> = None;
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            assert!(
                stdout.contains("not installed"),
                "an absent library must be REPORTED:\n{stdout}"
            );
            return Ok(());
        }
        for line in expect {
            assert!(
                stdout.contains(line),
                "{backend}: expected `{line}` in:\n{stdout}"
            );
        }
        match &first {
            None => first = Some(stdout),
            Some(f) => assert_eq!(
                answer_without_housekeeping(f),
                answer_without_housekeeping(&stdout),
                "both backends, one registry"
            ),
        }
    }
    Ok(())
}

/// @PLN133 S9 — the same lazy read, down core's Rust source and down a LOFT
/// driver, over one database.
///
/// Core drives sqlite in Rust (`sql_source.rs` + `sql_query.rs`, 913 lines) and
/// the loft library drives four backends behind one `SqlDb` interface. S10
/// deletes the Rust; this is the measurement that would let it. The claim is not
/// "the loft driver works" — it is that **the two paths are indistinguishable to
/// the program above them**.
///
/// Two element types of the same shape over two identical tables, in ONE program
/// bound to ONE connection string: `S9Rust` has no driver so core serves it,
/// `S9Loft` has one and S9's precedence rule sends it to loft. Every assertion is
/// made twice, once per path.
///
/// **The counts are the oracle, not the values.** A lazy read that fetched the
/// whole table would return exactly these values; only the trip count separates
/// it from an eager load, which is why @PLN129 asserts it. Here it is visible
/// from outside because the driver prints: three lookups reach the source and the
/// repeat of a resident key reaches none.
///
/// Nothing in the driver names a column. The table, the columns and the `WHERE`
/// all come from `derive(type_of(coll))` — the same `TableDef` a writer would
/// `render` into `CREATE TABLE` — which is requirement 2's one derivation doing
/// both jobs.
/// @PLN203 C8 — a lazy `sqlite:` source rebound to a shorter one is read as itself
/// (`tests/fixtures/sqldb/rebind_shorter.loft`).  The C runtime's SQL driver took the binding's
/// length-counted source text as a C string, and a loft text carries no terminator: it read on
/// into the buffer's old bytes.  On Windows that was this file's s9 test failing now and then
/// with "unable to open database file" (`…/pln133_s9.dble`); after a rebind to a shorter source
/// the leftover bytes are the longer source's tail on every host, so the C opened
/// `rebind_shorter.db-a-longer-name-nothing-opens` and answered `<none>`.  The interpreter
/// answers `grace`, and the C must too.
#[test]
fn a_lazy_source_rebound_shorter_is_read_as_itself() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("rebind_shorter.loft");
    for backend in ["--interpret", "--native-c"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            assert!(
                stdout.contains("not installed"),
                "an absent library must be REPORTED:\n{stdout}"
            );
            return Ok(());
        }
        assert!(
            stdout.contains("rebind nm=grace clean=[]"),
            "{backend}: the shorter source read as itself:\n{stdout}"
        );
    }
    Ok(())
}

#[test]
fn a_lazy_read_gives_one_answer_down_rust_and_down_loft() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("s9_two_paths.loft");

    // Each token is a claim about BOTH paths at once, so a difference between
    // them shows up as an asymmetric line rather than as an absent one:
    //
    //   value rust=grace loft=grace   the derived SELECT found the same row.
    //   float rust=0.25 loft=0.25     sqlite renders a REAL as %!.15g, so a
    //           SELECT that did not wrap the column comes back almost right.
    //           `select_by_key` applies the dialect's read expression; core's
    //           Rust source does its own equivalent, and they must agree.
    //   identity rust=true loft=true  one record however it is reached — which is
    //           what makes the collection, not an identity map, the authority.
    //   resident 1/1 then touched 2/2  `len` counts what was TOUCHED, so an
    //           eager load would read 3 here on either path.
    //   absent true/true + clean [] [] a genuine absence is not a failure, and
    //           reporting it as one is the mirror of the bug arc C exists for.
    let expect = [
        "value rust=grace loft=grace",
        "float rust=0.25 loft=0.25",
        "identity rust=true loft=true",
        "resident rust=1 loft=1",
        "second rust=alan loft=alan",
        "touched rust=2 loft=2",
        "absent rust=true loft=true",
        "clean rust=[] loft=[]",
        "s9 ok",
    ];

    let mut first: Option<String> = None;
    // `--static` too (@PLN197): the `sqlite` fixture's shim is linked INTO that binary, and
    // the package's optional library makes every one of its symbols lazy — a symbol the
    // shim defines must still be reached, not looked up by a name nothing exported.
    for (backend, link) in [
        ("--interpret", None),
        ("--native", None),
        ("--native", Some("--static")),
    ] {
        let backend = link.map_or_else(|| backend.to_string(), |l| format!("{backend} {l}"));
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .args(backend.split(' '))
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            assert!(
                stdout.contains("not installed"),
                "an absent library must be REPORTED:\n{stdout}"
            );
            return Ok(());
        }
        for line in expect {
            assert!(
                stdout.contains(line),
                "{backend}: expected `{line}` in:\n{stdout}"
            );
        }
        // THE oracle. Three lookups reach the loft driver — 42, 7 and the absent
        // 999 — and the repeat of 42 does not, because it hit the working set.
        // Every value above would be identical under an eager load; only this
        // would not.
        assert_eq!(
            stdout.matches("loft-driver key=").count(),
            3,
            "{backend}: 3 lookups reach the driver and a resident hit reaches \
             none:\n{stdout}"
        );
        match &first {
            None => first = Some(stdout),
            Some(f) => assert_eq!(
                answer_without_housekeeping(f),
                answer_without_housekeeping(&stdout),
                "both backends, one answer per path"
            ),
        }
    }
    Ok(())
}

/// @PLN133 S14 — THE GATE. Write rows, bind lazily to the SAME connection
/// string, traverse, and get back what was written.
///
/// The plan's three requirements as one program: one string switches every
/// consumer; a structure written is immediately readable through lazy loading;
/// and a loft type has ONE table definition — created where the database has
/// nothing, FOLLOWED where it already holds a table.
///
/// **It runs TWICE, and the second run is the one that matters.** Run 1 goes
/// into an empty database, where loft writes the schema. Run 2 goes into a table
/// made by hand — different column ORDER, the float kept in a `VARCHAR`, and an
/// extra column loft knows nothing about — where loft must follow it. Run 1
/// passes even if `reconcile` is a stub that always agrees; only run 2 proves
/// requirement 3.
///
/// **The trip count is the oracle the values cannot be.** A reader that fetched
/// the whole table would return exactly these rows. Only the number of trips
/// separates a lazy read from an eager one, so the driver prints one line per
/// trip: 42 reaches the database, the repeat of 42 does not, 7 does, and the
/// absent 999 does — three per run, six in all.
///
/// CI reaches sqlite only. `LOFT_SQLDB_MODE` selects the other three, and the
/// local four-backend run is written into the plan where it was measured
/// (doc/claude/TEST_ENVIRONMENTS.md § Database backends).
#[test]
fn a_structure_written_is_immediately_readable_through_one_connection_string() -> std::io::Result<()>
{
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("round_trip.loft");

    // Each token is one claim, and the `created`/`followed` prefix says which of
    // the two runs made it — so a design that only works on a table loft wrote
    // itself shows up as HALF the lines rather than as none.
    //
    //   value=grace float=0.25 flag=false  the row came back through a SELECT
    //           built from the same TableDef the INSERT was built from.
    //   identity=true                      one record however it is reached: a
    //           write through the second handle is visible through the first.
    //   resident=1 then touched=2          `len` counts what was TOUCHED, so an
    //           eager load would read 3.
    //   absent=true clean=[]               a genuine absence is not a failure —
    //           the mirror of the bug @PLN129 arc C exists for.
    //   digest=true                        @PLN23 S5: the record that came back
    //           IS the record that went in, compared through the same walk that
    //           WROTE it.  It sees what the tokens beside it cannot — those name
    //           only grace's fields and alan's NAME, so a driver returning
    //           `flag=false` for every row keeps every other claim (grace's flag
    //           genuinely is false) and fails on alan's digest alone.  Measured:
    //           that break leaves `created digest=true` and turns
    //           `created second=alan touched=2 digest=true` false.
    let expect = [
        "created value=grace float=0.25 flag=false",
        "created digest=true",
        "created identity=true resident=1",
        "created second=alan touched=2 digest=true",
        "created absent=true clean=[]",
        "followed value=grace float=0.25 flag=false",
        "followed digest=true",
        "followed identity=true resident=1",
        "followed second=alan touched=2 digest=true",
        "followed absent=true clean=[]",
        "round_trip ok",
    ];

    let mut first: Option<String> = None;
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            assert!(
                stdout.contains("not installed") || stdout.contains("cannot"),
                "an unreachable backend must be REPORTED:\n{stdout}"
            );
            return Ok(());
        }
        for line in expect {
            assert!(
                stdout.contains(line),
                "{backend}: expected `{line}` in:\n{stdout}"
            );
        }
        assert_eq!(
            stdout.matches("  trip key=").count(),
            6,
            "{backend}: three trips per run and none for a resident hit:\n{stdout}"
        );
        match &first {
            None => first = Some(stdout),
            Some(f) => assert_eq!(
                answer_without_housekeeping(f),
                answer_without_housekeeping(&stdout),
                "both backends, one round trip"
            ),
        }
    }
    Ok(())
}

/// @PLN23 S3 — the cursor model: a real result set, walked through a shim loft
/// compiled itself, with SQL NULL kept distinct from the empty string.
///
/// The whole vertical slice in one test — loft → libmariadb + a loft-built
/// ANSI-C shim → a live server → rows back — with no rustc anywhere.
///
/// **The `shim` mode is what keeps this honest.** The cursor needs a server, and
/// a test that merely skips when one is absent cannot tell "no database" from "a
/// dead binding", so it would report a broken shim as a pass on every machine
/// without MariaDB. The `shim` mode needs no server and still exercises the
/// pieces that can silently rot: the shim compiled, `char *` → `text?`, and a
/// NULL pointer arriving as loft null. Only the SERVER half is conditional.
#[test]
fn a_sql_cursor_walks_real_rows_and_keeps_null_apart_from_empty() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let present = [
        "/lib/x86_64-linux-gnu/libmariadb.so.3",
        "/usr/lib/libmariadb.so.3",
    ]
    .iter()
    .any(|p| fa::exists(std::path::Path::new(p)));
    if !present
        || loft::platform::process::harness_command("cc")
            .arg("--version")
            .output()
            .is_err()
    {
        return Ok(());
    }
    let libdir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // Beside the fixture it needs, NOT in tests/scripts/ — that directory is swept
    // and every script there must run standalone, while this one needs `--lib`.
    let script = libdir.join("mariadb").join("cursor.loft");
    let run = |backend: &str, mode: &str| -> std::io::Result<String> {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .env("LOFT_PLN23_MODE", mode)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // Exit status in the message, for the same reason as the sites above: a
        // Windows binary that cannot find a DLL dies before `main` with both
        // streams empty, and without the code the message says nothing at all.
        assert!(
            out.status.success(),
            "{backend}/{mode} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };

    // Always: the shim and the null crossing, no server involved.
    let shim = run("--interpret", "shim")?;
    assert!(
        shim.contains("shim null=true") && shim.contains("shim info=true"),
        "the shim must build and a null row must cross as loft null:\n{shim}"
    );
    assert_eq!(
        run("--native", "shim")?,
        shim,
        "both backends, one loft-built shim"
    );

    // Conditional: the cursor itself.
    let cursor = run("--interpret", "cursor")?;
    if cursor.contains("SKIP unreachable") {
        return Ok(()); // no server here; the shim half above still ran
    }
    for want in [
        "cols 3 rows 3",
        "row 1 ada NULL", // SQL NULL
        "row 2 grace [hi]",
        "row 3 kay []", // the empty string — NOT the same thing
        "done",
    ] {
        assert!(cursor.contains(want), "cursor missing `{want}`:\n{cursor}");
    }
    assert_eq!(
        run("--native", "cursor")?,
        cursor,
        "both backends must read the same rows"
    );
    Ok(())
}

/// @PLN23 S2 — the opaque-handle lifecycle against a real client library.
///
/// `MYSQL *` crosses as a loft `integer` holding the pointer, which is the
/// convention @PLN24 chose because loft has no type separating a handle from a
/// number. Three things have to hold: a handle comes back, it survives being
/// passed BACK in, and a failure carries C's own message across.
///
/// Deliberately **not** gated on a running server. A test that skips when the
/// database is absent cannot tell "no server" from "the binding is broken", and
/// would report a dead binding as a pass on every machine without MariaDB. So
/// the cells here need no server at all: `mysql_init` and `mysql_close` are
/// client-side, and connecting to a port nothing listens on exercises the error
/// path — the binding is proved either way, and only the SPEED of the failure
/// depends on the environment.
#[test]
fn a_c_library_handle_survives_the_round_trip_and_carries_its_error() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let present = [
        "/lib/x86_64-linux-gnu/libmariadb.so.3",
        "/usr/lib/libmariadb.so.3",
    ]
    .iter()
    .any(|p| fa::exists(std::path::Path::new(p)));
    if !present {
        return Ok(());
    }
    let dir = std::env::temp_dir().join("loft_pln23_s2");
    let pkg = dir.join("mariadb").join("src");
    fa::create_dir_all(&pkg)?;
    fa::write(
        dir.join("mariadb").join("loft.toml"),
        "[library]\nname = \"mariadb\"\nversion = \"0.0.1\"\n\n[c]\nlibs = \"libmariadb.so.3\"\n",
    )?;
    fa::write(
        pkg.join("mariadb.loft"),
        // `unix_socket` is `integer`, not `text`: it has to be able to be NULL,
        // and loft text is non-null with no way to spell a null pointer.
        "pub fn db_init(h: integer) -> integer;  #c \"mysql_init\" \"void*(void*)\"\n\
         pub fn db_close(h: integer);            #c \"mysql_close\" \"void(void*)\"\n\
         pub fn db_errno(h: integer) -> integer; #c \"mysql_errno\" \"int(void*)\"\n\
         pub fn db_error(h: integer) -> text;    #c \"mysql_error\" \"const char*(void*)\"\n\
         pub fn db_connect(h: integer, host: text, user: text, pass: text, db: text, port: integer, sock: integer, flags: integer) -> integer;\n\
         #c \"mysql_real_connect\" \"void*(void*, const char*, const char*, const char*, const char*, int, const char*, long)\"\n",
    )?;
    let prog = dir.join("s2.loft");
    fa::write(
        &prog,
        "use mariadb::*;\n\
         fn main() {\n\
         \x20 h = db_init(0);\n\
         \x20 println(\"handle {h != 0}\");\n\
         \x20 // Port 1 has no MariaDB anywhere; the point is the error crossing.\n\
         \x20 c = db_connect(h, \"127.0.0.1\", \"nobody\", \"nothing\", \"\", 1, 0, 0);\n\
         \x20 println(\"refused {c == 0} errno {db_errno(h) != 0} msg {db_error(h).len() > 0}\");\n\
         \x20 db_close(h);\n\
         \x20 println(\"closed\");\n\
         }\n",
    )?;
    let run = |backend: &str| -> std::io::Result<String> {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&dir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // The EXIT STATUS belongs in the message, because the interesting Windows
        // failure is a SILENT one: a binary that links but cannot find its DLL at
        // load time dies with `STATUS_DLL_NOT_FOUND` (0xC0000135) having written
        // nothing at all, so stdout and stderr are both empty and the assertion
        // said nothing without this.
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    for want in [
        "handle true",                      // a pointer came back as an integer
        "refused true errno true msg true", // and C's own diagnosis came back with it
        "closed",                           // the handle was still usable at the end
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    assert_eq!(run("--native")?, interp, "both backends, one C library");
    Ok(())
}

/// @PLN24 arc F / @PLN23 S1 — a `#c` binding reaches a real SYSTEM library, on
/// both backends, with no rustc and no dev headers.
///
/// The fixture proved the mechanism; a system library proves the parts a fixture
/// cannot have. `libmariadb.so.3` is a VERSIONED soname, and that is the whole
/// point of this test: `-l dylib=mariadb` makes the linker look for
/// `libmariadb.so`, the `-dev` symlink, while the interpreter `dlopen`s
/// `libmariadb.so.3`, the runtime file — so one declaration resolved to two
/// different files and the program ran interpreted and failed to LINK natively on
/// a machine where the library is plainly installed.
///
/// Skips when libmariadb is absent, because a machine without it says nothing.
#[test]
fn a_c_binding_reaches_a_versioned_system_library_on_both_backends() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // Present only if the runtime package is installed; the `-dev` symlink is
    // deliberately NOT required, which is the property under test.
    let present = [
        "/lib/x86_64-linux-gnu/libmariadb.so.3",
        "/usr/lib/libmariadb.so.3",
    ]
    .iter()
    .any(|p| fa::exists(std::path::Path::new(p)));
    if !present {
        return Ok(());
    }
    let dir = std::env::temp_dir().join("loft_pln23_s1");
    let pkg = dir.join("mariadb").join("src");
    fa::create_dir_all(&pkg)?;
    fa::write(
        dir.join("mariadb").join("loft.toml"),
        "[library]\nname = \"mariadb\"\nversion = \"0.0.1\"\n\n[c]\nlibs = \"libmariadb.so.3\"\n",
    )?;
    fa::write(
        pkg.join("mariadb.loft"),
        "pub fn client_info() -> text;       #c \"mysql_get_client_info\" \"const char*(void)\"\n\
         pub fn client_version() -> integer; #c \"mysql_get_client_version\" \"long(void)\"\n",
    )?;
    let prog = dir.join("s1.loft");
    fa::write(
        &prog,
        "use mariadb::*;\nfn main() { println(\"{client_info()} {client_version()}\") }\n",
    )?;

    let run = |backend: &str| -> std::io::Result<String> {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&dir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // The EXIT STATUS belongs in the message, because the interesting Windows
        // failure is a SILENT one: a binary that links but cannot find its DLL at
        // load time dies with `STATUS_DLL_NOT_FOUND` (0xC0000135) having written
        // nothing at all, so stdout and stderr are both empty and the assertion
        // said nothing without this.
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    // Not an exact version — that is the machine's. What is pinned: the text came
    // back non-empty and the integer is a real version, so both crossings worked.
    let v: i64 = interp
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    assert!(
        v > 10000,
        "the client version must cross as a real number: {interp:?}"
    );
    assert_eq!(
        run("--native")?,
        interp,
        "both backends, one system library"
    );
    Ok(())
}

/// A wasm target gets a NAMED refusal, and only where a `#c` binding is actually called.
///
/// A wasm module cannot open a shared library, so a `#c` call there is either a symbol the
/// WASI libc happens to export — which links and traps, wasm32 being a third data model
/// (ILP32) — or one it does not export, which fails at the link naming no package.  Neither
/// tells a reader what is wrong, so the C backend refuses the reachable call by name at
/// emission, before any wasm toolchain is asked for — the guarantee is checked on every
/// machine that runs the suite.  The end-to-end halves live in `html_wasm.rs`.
///
/// Three parts: on the HOST target the same program builds and answers (the calibration —
/// a refusal that fired everywhere would otherwise read as a pass); on wasm the called
/// binding is refused naming the function, the symbol, the target and its flag; and the
/// binding that is only declared is not named.
#[test]
fn a_c_binding_is_refused_by_name_on_a_wasm_target() -> std::io::Result<()> {
    let dir = std::env::temp_dir().join(format!("loft_pln24_arce_{}", std::process::id()));
    fa::create_dir_all(&dir)?;
    let src_path = dir.join("arce.loft");
    fa::write(
        &src_path,
        // `used` is called; `unused` is only declared.  Both are `#c`.
        "fn used(s: text) -> integer;    #c \"strlen\" \"size_t(const char*)\"\n\
         fn unused(v: integer) -> integer; #c \"abs\" \"int(int)\"\n\
         fn main() { println(\"{used(\\\"hello\\\")}\") }\n",
    )?;
    let loft = |args: &[&std::ffi::OsStr]| {
        loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .args(args)
            .arg(&src_path)
            .env("LOFT_TIMEOUT", "300")
            .output()
    };

    let host = loft(&["--native".as_ref()])?;
    let host_out = String::from_utf8_lossy(&host.stdout);
    assert!(
        host.status.success() && host_out.trim() == "5",
        "the host target must build the binding and call it: {host_out}\n{}",
        String::from_utf8_lossy(&host.stderr)
    );

    let wasm_out = dir.join("arce.wasm");
    let wasm = loft(&[
        "--native".as_ref(),
        "--native-wasm".as_ref(),
        wasm_out.as_os_str(),
    ])?;
    let refusal = String::from_utf8_lossy(&wasm.stderr);
    assert!(
        !wasm.status.success(),
        "a wasm build of a called `#c` binding must be refused: {refusal}"
    );
    assert!(
        refusal.contains("`used` is bound to the C symbol 'strlen' with #c")
            && refusal.contains("wasm (wasip2)")
            && refusal.contains("--native-wasm")
            && refusal.contains("cannot open a shared library"),
        "the reachable call must be refused by name: {refusal}"
    );
    assert!(
        !refusal.contains("`unused`"),
        "a binding that is only declared must not be refused: {refusal}"
    );
    let _ = fa::remove_dir_all(&dir);
    Ok(())
}

/// @PLN24 arc D — loft compiles the ANSI-C shim itself, with `cc` and no rustc.
///
/// The plan's trade is that loft-core stays a generic linking tool and every
/// signature the fixed trampolines cannot express is wrapped in a few lines of C
/// the library ships. That trade only holds if loft can BUILD those lines —
/// otherwise the escape hatch is a claim, and the author's alternative is the
/// rustc toolchain `#c` exists to avoid.
///
/// One cell per shape that needs a shim: a `double` argument (a different
/// register file, so it crosses as its bit pattern), an out-parameter (two
/// answers, one return slot), and a caller-frees `char *` (loft never frees one,
/// so the shim owns the release). The float cell is hand-computed rather than
/// copied from a run — 2.5 × 4.0 is exactly 10.0, whose bit pattern is pinned
/// below, so a shim that returned a plausible-but-wrong double fails.
#[test]
fn loft_builds_the_ansi_c_shim_a_package_ships() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi");
    let libdir = root.join("pkg");
    // Start from no artifact, so the build itself is what is under test.
    let _ = fa::remove_dir_all(libdir.join("lcshim").join("native-auto"));

    let prog = std::env::temp_dir().join("loft_pln24_shim.loft");
    fa::write(
        &prog,
        "use lcshim::*;\n\
         fn main() {\n\
         \x20 println(\"scale {shim_scale(4612811918334230528, 4616189618054758400)}\");\n\
         \x20 println(\"mod {shim_mod(17, 5)}\");\n\
         \x20 println(\"owned {shim_owned(7)}\");\n\
         }\n",
    )?;
    let run = |backend: &str| -> std::io::Result<String> {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // The EXIT STATUS belongs in the message, because the interesting Windows
        // failure is a SILENT one: a binary that links but cannot find its DLL at
        // load time dies with `STATUS_DLL_NOT_FOUND` (0xC0000135) having written
        // nothing at all, so stdout and stderr are both empty and the assertion
        // said nothing without this.
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    for want in [
        // 2.5 * 4.0 = 10.0 → 0x4024000000000000.
        "scale 4621819117588971520",
        "mod 2",
        "owned shim-7",
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    // The artifact is the proof loft did the compiling: nothing else put it there.
    let built: Vec<_> = fa::read_dir(libdir.join("lcshim").join("native-auto"))
        .map(|d| d.into_iter().filter_map(|e| e.os_name()).collect())
        .unwrap_or_default();
    assert!(
        !built.is_empty(),
        "loft must have built the shim into native-auto/"
    );

    let native = run("--native")?;
    assert_eq!(
        interp, native,
        "a shim-backed binding must answer the same on both backends"
    );
    Ok(())
}

/// loft#2234 — `--tests` opens the C libraries of a library reached through a RELATIVE
/// `--lib`, on both backends, as a plain run does.  Two kinds: a `[c] shim` loft builds, and
/// a library shipped beside its `loft.toml` (`[c] libs`).  Both failed on main: the test
/// runner compiled inside the test file's directory, so a path recorded relative to where
/// loft was started no longer resolved, and the shim's path doubled its package directory.
/// `test_marker` holds the other half: the RUN still stands in the test file's directory,
/// where a C `fopen("marker.txt")` finds the file beside the test.
#[test]
fn a_test_run_opens_a_relative_lib_librarys_c_libraries() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let tree = std::env::temp_dir().join(format!("loft_2234_{}", std::process::id()));
    let _ = fa::remove_dir_all(&tree);
    let manifest = |name: &str, c: &str| {
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nloft = \">=2026.10.0\"\n\n\
             [library]\nentry = \"src/{name}.loft\"\n\n[c]\n{c}\n"
        )
    };
    let twice = tree.join("libs").join("twice");
    fa::create_dir_all(twice.join("c"))?;
    fa::create_dir_all(twice.join("src"))?;
    fa::write(
        twice.join("loft.toml"),
        manifest("twice", "shim = \"c/twice.c\""),
    )?;
    fa::write(
        twice.join("c").join("twice.c"),
        "#include <stdint.h>\n#include <stdio.h>\n\
         int64_t tw_twice(int64_t v);\nint64_t tw_twice(int64_t v) { return v * 2; }\n\
         int64_t tw_marker(void);\nint64_t tw_marker(void) {\n\
         \x20 FILE *f = fopen(\"marker.txt\", \"r\");\n\
         \x20 if (!f) return 0;\n  fclose(f);\n  return 1;\n}\n",
    )?;
    fa::write(
        twice.join("src").join("twice.loft"),
        "fn tw_twice(v: integer) -> integer;\n#c \"tw_twice\" \"int64_t(int64_t)\"\n\
         fn tw_marker() -> integer;\n#c \"tw_marker\" \"int64_t(void)\"\n\
         pub fn twice(v: integer) -> integer { tw_twice(v) }\n\
         pub fn marker() -> integer { tw_marker() }\n",
    )?;
    let beside = tree.join("libs").join("beside");
    fa::create_dir_all(beside.join("src"))?;
    // The host's spelling of a shared library: a Windows program finds a DLL only beside it,
    // and the driver stages one there by its `.dll` name.
    let lib = format!("libbeside_c.{}", loft::platform::dll_extension());
    fa::write(
        beside.join("loft.toml"),
        manifest("beside", &format!("libs = [\"{lib}\"]")),
    )?;
    let c_src = tree.join("beside.c");
    fa::write(
        &c_src,
        "#include <stdint.h>\nint64_t bs_three(int64_t v);\n\
         int64_t bs_three(int64_t v) { return v * 3; }\n",
    )?;
    let built = loft::platform::process::harness_command("cc")
        .args(["-shared", "-fPIC"])
        .arg(&c_src)
        .arg("-o")
        .arg(beside.join(&lib))
        .status()?;
    assert!(built.success(), "cc could not build the beside library");
    fa::write(
        beside.join("src").join("beside.loft"),
        "fn bs_three(v: integer) -> integer;\n#c \"bs_three\" \"int64_t(int64_t)\"\n\
         pub fn three(v: integer) -> integer { bs_three(v) }\n",
    )?;
    fa::create_dir_all(tree.join("tests"))?;
    fa::write(tree.join("tests").join("marker.txt"), "")?;
    fa::write(
        tree.join("tests").join("uses.loft"),
        "use twice;\nuse beside;\n\
         fn test_twice() { assert(twice::twice(21) == 42, \"twice\"); }\n\
         fn test_three() { assert(beside::three(5) == 15, \"three\"); }\n\
         fn test_marker() { assert(twice::marker() == 1, \"the run stands in tests/\"); }\n",
    )?;
    struct Leg {
        name: &'static str,
        flag: Option<&'static str>,
        recompile: bool,
    }
    let legs = [
        Leg {
            name: "interpreter",
            flag: None,
            recompile: false,
        },
        Leg {
            name: "per-test compile",
            flag: None,
            recompile: true,
        },
        Leg {
            name: "native",
            flag: Some("--native"),
            recompile: false,
        },
    ];
    for Leg {
        name: leg,
        flag,
        recompile,
    } in legs
    {
        let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
        cmd.current_dir(&tree)
            .args(["--lib", "libs", "--tests", "tests/uses.loft"])
            .args(flag);
        if recompile {
            cmd.env("LOFT_TEST_RECOMPILE", "1");
        }
        let out = cmd.output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success() && stdout.contains("3 passed"),
            "{leg}: exited {}\nstdout={stdout}\nstderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = fa::remove_dir_all(&tree);
    Ok(())
}

/// loft#2238 — an installed optional library answers `c_library_available` true on both
/// backends and both links when its package also ships a `[c] shim`.  The symbols a package
/// declares are attributed to its one optional library, and the shim's own were among them;
/// a native program cannot find those by name (linked in under a static link, not yet open
/// under a dynamic one), so it answered `false` where the interpreter answered `true`.
/// The optional library is the host's C runtime (`platform::host_c_runtime_library`), which
/// every host has; the absent one is the control, which must stay `false`.
#[test]
fn an_installed_optional_library_beside_a_shim_is_available_on_both_backends() -> std::io::Result<()>
{
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let tree = std::env::temp_dir().join(format!("loft_2238_{}", std::process::id()));
    let _ = fa::remove_dir_all(&tree);
    let package = |name: &str, optional: &str, shim_c: &str, loft: &str| -> std::io::Result<()> {
        let dir = tree.join("libs").join(name);
        fa::create_dir_all(dir.join("src"))?;
        fa::write(
            dir.join("loft.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nloft = \">=2026.10.0\"\n\n\
                 [library]\nentry = \"src/{name}.loft\"\n\n[c]\n\
                 optional-libs = \"{optional}\"\nshim = \"src/shim.c\"\n"
            ),
        )?;
        fa::write(dir.join("src").join("shim.c"), shim_c)?;
        fa::write(dir.join("src").join(format!("{name}.loft")), loft)
    };
    let crt = loft::platform::host_c_runtime_library();
    package(
        "withc",
        crt,
        "#include <stdint.h>\nint64_t wc_seven(void);\nint64_t wc_seven(void) { return 7; }\n",
        "fn wc_seven() -> integer;\n#c \"wc_seven\" \"int64_t(void)\"\n\
         fn c_abs(v: integer) -> integer;\n#c \"abs\" \"int(int)\"\n\
         pub fn seven() -> integer { wc_seven() }\n\
         pub fn absolute(v: integer) -> integer { c_abs(v) }\n",
    )?;
    package(
        "absent",
        "libloft_2238_absent.so.9",
        "#include <stdint.h>\nint64_t ab_one(void);\nint64_t ab_one(void) { return 1; }\n",
        "fn ab_one() -> integer;\n#c \"ab_one\" \"int64_t(void)\"\n\
         fn ab_missing() -> integer;\n#c \"loft_2238_missing\" \"int64_t(void)\"\n\
         pub fn one() -> integer { ab_one() }\n",
    )?;
    let prog = tree.join("p.loft");
    fa::write(
        &prog,
        format!(
            "use withc;\nuse absent;\nfn main() {{\n\
             \x20 c = c_library_available(\"{crt}\");\n\
             \x20 a = c_library_available(\"libloft_2238_absent.so.9\");\n\
             \x20 println(\"libc {{c}} absent {{a}} one {{absent::one()}}\");\n}}\n"
        ),
    )?;
    // Nothing of `withc` is called: a dynamic program binds only the packages it reaches, so
    // its shim is not open when the question is asked — the dynamic half of the defect.  A
    // program that calls into the package after asking binds it at start and was right.
    let want = "libc true absent false one 1\n";
    let legs = [
        ("--interpret", "1"),
        ("--interpret", "0"),
        ("--native", "1"),
        ("--native", "0"),
    ];
    let mut wrong = Vec::new();
    for (backend, cabi) in legs {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .current_dir(&tree)
            .args([backend, "--lib", "libs", "p.loft"])
            .env("LOFT_NATIVE_CABI", cabi)
            .output()?;
        let got = String::from_utf8_lossy(&out.stdout);
        if got != want {
            wrong.push(format!(
                "{backend} LOFT_NATIVE_CABI={cabi}: {got:?}, exited {}\n{}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    assert!(wrong.is_empty(), "want {want:?}:\n{}", wrong.join("\n"));
    let _ = fa::remove_dir_all(&tree);
    Ok(())
}

/// @PLN199 B2 — a loft function that calls a `#c` binding compiles into its library's
/// cdylib: the gate used to turn the binding into an interpreted CALLEE, so every loft
/// wrapper of a C function ran interpreted.  The cdylib is linked against no C library — its
/// host has bound them — so each symbol resolves at its first call (`PackageLink::Library`).
/// The trace must count both wrappers compiled, and the bindings neither compiled nor
/// interpreted; the answers are hand-computed (17 mod 5 = 2, plus 100; the shim answers
/// `shim-<n>`) and must hold interpreted, mixed, `--native` and `--native --static`.
#[test]
fn a_loft_wrapper_over_a_c_binding_compiles() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        println!("a_loft_wrapper_over_a_c_binding_compiles: skipped (no cc)");
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi");
    let libdir = root.join("pkg");
    let _ = fa::remove_dir_all(libdir.join("lcshim").join("native-auto"));
    let prog = std::env::temp_dir().join(format!("loft_pln199_b2_{}.loft", std::process::id()));
    fa::write(
        &prog,
        "use lcshim::*;\nfn main() {\n  println(\"{wrapped_mod(17, 5)} {wrapped_owned(7)} {shim_mod(17, 5)}\");\n}\n",
    )?;
    let run = |args: &[&str], env: &[(&str, &str)]| -> std::io::Result<(String, String)> {
        let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
        cmd.args(args)
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "600");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success(),
            "{args:?} exited {}: stdout={stdout:?} stderr={stderr}",
            out.status
        );
        Ok((stdout, stderr))
    };
    let want = "102 <shim-7> 2\n";
    let (interp, _) = run(&["--interpret"], &[("LOFT_NO_NATIVE_LIBS", "1")])?;
    assert_eq!(interp, want, "interpreted");
    let (mixed, trace) = run(&["--interpret"], &[("LOFT_TRACE_NATIVE_GATE", "1")])?;
    assert_eq!(mixed, want, "mixed:\n{trace}");
    assert!(
        trace.contains("lcshim: 2 of 2 callable functions compiled"),
        "both wrappers compiled, the bindings not counted:\n{trace}"
    );
    let (native, _) = run(&["--native"], &[])?;
    assert_eq!(native, want, "--native");
    let (stat, _) = run(&["--native", "--static"], &[])?;
    assert_eq!(stat, want, "--native --static");
    let _ = fa::remove_file(&prog);
    Ok(())
}

/// @PLN24 arc D — the return spellings a `#c` declaration must refuse, because
/// nothing at runtime would.
///
/// The plan's central measurement is that a wrong binding produces a plausible
/// answer rather than a failure, so every one of these is caught at the
/// declaration or not at all. Both backends must refuse in the same words: a
/// shape rejected by one and accepted by the other is the divergence this plan
/// exists to keep out.
#[test]
fn a_text_return_must_say_it_is_a_c_string() -> std::io::Result<()> {
    // (declaration, the words the refusal has to carry)
    let cases = [
        (
            "pub fn f() -> text;   #c \"lc_x\" \"void*(void)\"",
            "spell the return `char*`",
        ),
        (
            "pub fn f() -> text;   #c \"lc_x\" \"int(void)\"",
            "the loft declaration returns `text`",
        ),
        (
            // A `vector` return cannot be reconstructed from a pointer: C
            // carries no length, which is why an ARGUMENT crosses as a PAIR and
            // a return cannot cross at all.
            "pub fn f() -> vector<integer>;   #c \"lc_x\" \"char*(void)\"",
            "returns `vector<integer>` but the C signature returns `char *`",
        ),
    ];
    for (decl, want) in cases {
        let path = std::env::temp_dir().join("loft_pln24_c_textret_refuse.loft");
        fa::write(&path, format!("{decl}\nfn main() {{ println(\"x\") }}\n"))?;
        let mut seen = Vec::new();
        for backend in ["--interpret", "--native"] {
            let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
                .arg(backend)
                .arg("--errors=compact")
                .arg(&path)
                .output()?;
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            assert!(
                !out.status.success(),
                "{backend} must refuse `{decl}`: {stderr}"
            );
            assert!(
                stderr.contains(want),
                "{backend}: refusing `{decl}` must say `{want}`: {stderr}"
            );
            seen.push(stderr);
        }
        assert_eq!(
            seen[0], seen[1],
            "and both backends must say it identically"
        );
    }
    Ok(())
}

/// @PLN128 arc E — a `vector<T>` and the C pointee are two spellings of ONE
/// layout, and a declaration where they disagree is refused.
///
/// Every case here was a running program with wrong numbers in it, measured
/// against real OpenBLAS on both backends with exit 0 and no diagnostic:
///
/// * `vector<integer>` (8-byte) against LAPACK's `int *` pivot array — `dgesv_`
///   answered `ipiv = 8589934593, 0` where the pivots are `1, 2`.
/// * `vector<single>` (4-byte) against `double *` — `daxpy_` wrote 24 bytes
///   into a 12-byte loft vector, which is a write past the end of a store
///   allocation from a declaration loft accepted.
/// * `vector<i8>` / `vector<i16>` — loft stores narrow SIGNED elements as
///   `val - min`, so C reads every value shifted by 128 / 32768.
/// * `vector<text>` — the elements are loft's own heap handles, so C
///   dereferences a store offset as an address: immediate SIGSEGV.
///
/// None of them needs a library to be installed: the declaration is what is
/// wrong, so the refusal lands before anything is called. The counterpart —
/// every element width that DOES cross, checked against values C computes — is
/// in `numeric_array_shapes_cross_identically_on_both_backends`.
/// @C108 — a `vector<T>` and the C pointee are one layout.
#[test]
fn a_vector_element_must_match_the_c_pointee() -> std::io::Result<()> {
    // (declaration, the words the refusal has to carry)
    let cases = [
        (
            "pub fn f(v: vector<integer>);   #c \"lc_x\" \"void(int*)\"",
            "striding 4 bytes",
        ),
        (
            "pub fn f(v: vector<single>);   #c \"lc_x\" \"void(double*)\"",
            "striding 8 bytes",
        ),
        // Same WIDTH, different class. The message must not claim an overrun
        // here — there isn't one — only that every value arrives different.
        (
            "pub fn f(v: vector<float>);   #c \"lc_x\" \"void(const int64_t*)\"",
            "the same width read as integers",
        ),
        (
            "pub fn f(v: vector<i16>);   #c \"lc_x\" \"void(const short*)\"",
            "loft stores narrow SIGNED elements as `val - min`",
        ),
        (
            "pub fn f(v: vector<text>);   #c \"lc_x\" \"void(const char *const *)\"",
            "loft's own heap handles",
        ),
    ];
    for (decl, want) in cases {
        let path = std::env::temp_dir().join("loft_pln128_elem_refuse.loft");
        fa::write(&path, format!("{decl}\nfn main() {{ println(\"x\") }}\n"))?;
        let mut seen = Vec::new();
        for backend in ["--interpret", "--native"] {
            let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
                .arg(backend)
                .arg("--errors=compact")
                .arg(&path)
                .output()?;
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            assert!(
                !out.status.success(),
                "{backend} must refuse `{decl}`: {stderr}"
            );
            assert!(
                stderr.contains(want),
                "{backend}: refusing `{decl}` must say `{want}`: {stderr}"
            );
            seen.push(stderr);
        }
        assert_eq!(
            seen[0], seen[1],
            "and both backends must say it identically"
        );
    }
    // `void*` is the opaque escape hatch and stays open: it is how `write(2)`
    // takes a `vector<u8>`, and it is the author saying "these are bytes".
    let path = std::env::temp_dir().join("loft_pln128_elem_opaque.loft");
    fa::write(
        &path,
        "pub fn f(v: vector<float>);   #c \"lc_x\" \"void(const void*, int64_t)\"\n\
         fn main() { println(\"x\") }\n",
    )?;
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--errors=compact")
            .arg(&path)
            .output()?;
        assert!(
            out.status.success(),
            "{backend} must accept an opaque `void*` pointee: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

/// @PLN128 Q5 — a RETAINING C API is bindable, over a C-owned buffer.
///
/// The plan/execute split is not an FFTW quirk: zlib's `z_stream` keeps
/// `next_in`/`next_out`, `sqlite3_bind_text(SQLITE_STATIC)` keeps the caller's
/// bytes, and every "context object" API is this shape. C holds a buffer
/// pointer across two calls the caller makes.
///
/// Bound with a **loft** vector that is a use-after-free (E6b): loft frees the
/// vector at its last loft-visible use, which is the call that handed the
/// pointer over, and C reads whatever took its place. Measured, both backends,
/// no fault and no diagnostic — and deliberately NOT asserted here, because
/// pinning the current output would lock the bug in.
///
/// Bound with a **C-owned** buffer it is ordinary, which is what this pins.
/// Nothing loft owns is retained; the buffer's lifetime is the handle's; and
/// the two copies live only for their own call, which is what `#c` already
/// guarantees. It needs no shim — `memcpy` is libc — and it is what FFTW's own
/// documentation tells C callers to do anyway, because `fftw_malloc` is what
/// gives the SIMD alignment.
///
/// The expected value is `lc_selftest.c`'s, computed in C.
#[test]
fn a_retaining_c_api_binds_over_a_c_owned_buffer() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi");
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let built = loft::platform::process::harness_command("make")
        .arg("-C")
        .arg(&root)
        .output()?;
    assert!(
        built.status.success(),
        "the fixture library must build: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let prog = std::env::temp_dir().join("loft_pln128_retain.loft");
    fa::write(
        &prog,
        "use lcabi::*;\n\
         fn main() {\n\
         \x20 n = 3;\n\
         \x20 bytes = n * 8;\n\
         \x20 buf = lc_buf_alloc(bytes);\n\
         \x20 src: vector<float> = [1.5, 2.25, 4.0];\n\
         \x20 lc_load(buf, src, bytes);\n\
         // The plan RETAINS the buffer here and reads it in `lc_run` below —\n\
         // a later call, which is the shape that fails with a loft vector.\n\
         \x20 p = lc_plan(buf, n);\n\
         \x20 println(\"retained {lc_run(p)}\");\n\
         // And back out, so the round trip is closed rather than assumed.\n\
         \x20 back: vector<float> = [0.0, 0.0, 0.0];\n\
         \x20 lc_store(back, buf, bytes);\n\
         \x20 println(\"roundtrip {back[0]} {back[1]} {back[2]}\");\n\
         \x20 lc_plan_free(p);\n\
         \x20 lc_buf_free(buf);\n\
         }\n",
    )?;
    let libdir = root.join("pkg");
    let run = |backend: &str| -> std::io::Result<String> {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    for want in [
        // 1.5*1 + 2.25*2 + 4.0*3 = 18.0, scaled by 1000 — `lc_selftest.c`'s value.
        "retained 18000",
        "roundtrip 1.5 2.25 4",
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    let native = run("--native")?;
    assert_eq!(
        interp, native,
        "a retaining API must answer the same on both backends"
    );
    Ok(())
}

/// @PLN128 arc E — a float RETURN binds; a float ARGUMENT still does not.
///
/// The plan settled the two together and that was one decision too few. An
/// argument would need a trampoline per SUBSET of positions that are float —
/// `2^arity`, genuinely impossible — while the return is a single axis and
/// costs one more expansion of the same arity list. Refusing it cost the
/// level-1 BLAS *functions* (`ddot_`, `dnrm2_`, `dasum_`) and every LAPACK
/// auxiliary that answers a number, and the cure it prescribed was an ANSI-C
/// shim per routine: a C toolchain in the build of every numeric package.
///
/// The pairing is exact rather than widening: a C `float` leaves a SINGLE in
/// the return register, so binding it to loft `float` would read those bits as
/// a double and get a denormal.
#[test]
fn a_float_return_binds_and_a_float_argument_still_does_not() -> std::io::Result<()> {
    let cases = [
        (
            "pub fn f(v: vector<float>) -> integer;   #c \"lc_x\" \"double(const double*, int64_t)\"",
            Some("must return `float`, not `integer`"),
        ),
        (
            "pub fn f(v: vector<float>) -> float;   #c \"lc_x\" \"float(const double*, int64_t)\"",
            Some("must return `single`, not `float`"),
        ),
        (
            "pub fn f(v: integer) -> float;   #c \"lc_x\" \"int64_t(int64_t)\"",
            Some("a float comes back only from a C `float` or `double`"),
        ),
        (
            "pub fn f(v: float);   #c \"lc_x\" \"void(double)\"",
            Some("travels in an SSE register the caller does not write"),
        ),
        // The two that now bind. Nothing is called, so no library is needed —
        // the declaration either type-checks or it does not.
        (
            "pub fn f(v: vector<float>) -> float;   #c \"lc_x\" \"double(const double*, int64_t)\"",
            None,
        ),
        (
            "pub fn f(v: vector<single>) -> single;   #c \"lc_x\" \"float(const float*, int64_t)\"",
            None,
        ),
    ];
    for (decl, want) in cases {
        let path = std::env::temp_dir().join("loft_pln128_float_return.loft");
        fa::write(&path, format!("{decl}\nfn main() {{ println(\"x\") }}\n"))?;
        let mut seen = Vec::new();
        for backend in ["--interpret", "--native"] {
            let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
                .arg(backend)
                .arg("--errors=compact")
                .arg(&path)
                .output()?;
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            match want {
                Some(w) => {
                    assert!(
                        !out.status.success(),
                        "{backend} must refuse `{decl}`: {stderr}"
                    );
                    assert!(
                        stderr.contains(w),
                        "{backend}: refusing `{decl}` must say `{w}`: {stderr}"
                    );
                }
                None => assert!(
                    out.status.success(),
                    "{backend} must accept `{decl}`: {stderr}"
                ),
            }
            seen.push(stderr);
        }
        assert_eq!(
            seen[0], seen[1],
            "and both backends must answer `{decl}` identically"
        );
    }
    Ok(())
}

/// Unbounded recursion in a `--native` (C) program surfaces loft's typed `call stack
/// overflow` and a non-zero exit — the same cap the interpreter enforces — not a raw abort
/// of the process when the OS stack runs out.
#[test]
fn native_deep_recursion_reports_clean_stack_overflow() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // Written to a temp file, NOT tests/scripts/ (which the success-runners sweep — an
    // infinitely-recursing script would break them).
    let path = std::env::temp_dir().join("loft_native_stack_overflow_guard.loft");
    fa::write(
        &path,
        "fn recur(n: integer) -> integer { m = recur(n + 1); return m + 1; }\n\
         fn main() { x = recur(0); print(\"{x}\"); }\n",
    )?;
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--native")
        .arg(&path)
        .env("LOFT_TIMEOUT", "300")
        .output()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "deep recursion must exit non-zero; stderr: {stderr}"
    );
    assert!(
        stderr.contains("call stack overflow"),
        "expected loft's typed stack-overflow diagnostic; stderr: {stderr}"
    );
    assert!(
        !stderr.contains("fatal runtime error")
            && !stderr.contains("Segmentation fault")
            && !stderr.contains("Abort trap"),
        "must NOT be a raw stack-overflow abort; stderr: {stderr}"
    );
    Ok(())
}

/// Library packages whose tests do not compile or run under `--native`.
const LIB_PKGS_NATIVE_SKIP: &[&str] = &[
    // (empty) An entry here names a package whose tests do not compile or run
    // under `--native`, the open issue that explains it, and the condition that
    // removes it.
];

/// Specific library test FILES skipped under `--native` (the rest of the
/// package DOES compile), keyed `"<pkg>/<file>.loft"`.
const LIB_TESTS_NATIVE_SKIP: &[&str] = &[
    // (empty) The `web` fixture's network tests live in `tests-network/`, which no
    // suite walks (TESTING.md § Every skip says why, how it runs instead, and when
    // it ends) — nothing to list.  An entry here needs the open issue that explains
    // it and the condition that removes it.
];

/// True if `entry` (a `lib/<pkg>/tests/<file>.loft` path) is skipped under the
/// NATIVE library gate (its own codegen-gap list above; the interpreter gate's
/// skips live in `wrap.rs::lib_test_skipped`).
fn native_lib_test_skipped(entry: &Path) -> bool {
    let file = fa::file_name(entry).unwrap_or_default();
    let pkg = entry
        .parent()
        .and_then(|d| d.parent())
        .and_then(fa::file_name)
        .unwrap_or_default();
    if LIB_PKGS_NATIVE_SKIP.contains(&pkg.as_str()) {
        return true;
    }
    let key = format!("{pkg}/{file}");
    LIB_TESTS_NATIVE_SKIP.contains(&key.as_str())
}

/// Run `loft [extra_args] test <stem>` for a lib package in a UNIQUE temp CWD —
/// a `.loft_test_tmp_*` SIBLING inside `lib/` (so the package's relative deps
/// `../<name>` still resolve to the real packages), with the package's contents
/// symlinked in.  Isolates cwd-relative test artifacts so the native and
/// interpreter lib suites (separate, concurrently-run test binaries) don't race
/// on a shared file (e.g. `moros_render_test.glb`).  Duplicated from `wrap.rs`
/// (integration-test binaries can't share fns).  Non-unix falls back to `pkg_dir`.
fn run_lib_test_in_temp_cwd(
    loft_bin: &str,
    pkg_dir: &Path,
    stem: &str,
    extra_args: &[&str],
) -> std::io::Result<std::process::Output> {
    let mut args: Vec<&str> = extra_args.to_vec();
    args.push("test");
    args.push(stem);
    // approved exemption: the per-run cwd is built from symlinks, which need the symlink privilege on Windows; Windows substitute: the run in the package directory below
    #[cfg(unix)]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let lib_root = pkg_dir.parent().unwrap_or(pkg_dir);
        let tmp = lib_root.join(format!(
            ".loft_test_tmp_{}_{}",
            std::process::id(),
            CTR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fa::remove_dir_all(&tmp);
        fa::create_dir(&tmp)?;
        for entry in fa::read_dir(pkg_dir)? {
            let target = fa::try_plain_canonical(&entry).unwrap_or_else(|| entry.os_spelling());
            let _ = fa::symlink(&target, tmp.join(entry.os_name().unwrap_or_default()));
        }
        let out = loft::platform::process::harness_command(loft_bin)
            .current_dir(&tmp)
            .args(&args)
            .output();
        let _ = fa::remove_dir_all(&tmp);
        out
    }
    // approved exemption: the per-run cwd is built from symlinks, which need the symlink privilege on Windows; Windows substitute: the run in the package directory below
    #[cfg(not(unix))]
    {
        loft::platform::process::harness_command(loft_bin)
            .current_dir(pkg_dir)
            .args(&args)
            .output()
    }
}

/// loft#878 — a test file that names a helper the way its LIBRARY does must still
/// compile and run under `--native`.
///
/// A compiled program is one flat namespace, so two same-named fns from different files
/// need distinct names in it, and every call must name the one its source resolved to.
/// The colliding call here takes the adopt-or-copy bind — the callee returns a LOCAL bound
/// from another call — which is the shape that once named the wrong definition.
///
/// Both halves are asserted: the library's private `defaulted(W) -> boolean` is REACHED
/// (through `cell`, which the test calls), and the test-local `defaulted(integer) -> W`
/// is reached directly — so the guard fails if either definition goes missing, not only
/// if the name resolves wrongly.
// @speed 1.2
#[test]
fn a_test_local_name_shadowing_a_library_fn_compiles_natively_878() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("loft_878_{pid}"));
    let _ = fa::remove_dir_all(&root);
    let pkg = root.join("shadowlib");
    fa::create_dir_all(pkg.join("src"))?;
    fa::create_dir_all(pkg.join("tests"))?;
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"shadowlib\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
         [library]\nentry = \"src/shadowlib.loft\"\n",
    )?;
    fa::write(
        pkg.join("src/shadowlib.loft"),
        "pub struct W { pub w_n: integer, pub w_tag: text }\n\
         pub struct H { pub h_q: integer, pub h_r: integer }\n\
         pub fn make(n: integer) -> W { W { w_n: n, w_tag: \"lib\" } }\n\
         pub fn bump(w: W, n: integer) -> integer { w.w_n + n }\n\
         fn defaulted(w: W) -> boolean { w.w_n > 0 }\n\
         pub fn cell(w: W, q: integer) -> H {\n\
         \x20 if q < 0 {\n\
         \x20   if defaulted(w) { return H { h_q: w.w_n, h_r: 0 }; }\n\
         \x20   return H {};\n\
         \x20 }\n\
         \x20 return H { h_q: q, h_r: 1 };\n\
         }\n",
    )?;
    fa::write(
        pkg.join("tests/probe.loft"),
        "use shadowlib::*;\n\
         fn defaulted(h: integer) -> W {\n\
         \x20 w = make(h);\n\
         \x20 assert(bump(w, 0) == h, \"the fixture was built wrong: {w.w_n}\");\n\
         \x20 w\n\
         }\n\
         fn test_shadow() {\n\
         \x20 d = defaulted(3);\n\
         \x20 assert(d.w_n == 3, \"the test-local one answered {d.w_n}\");\n\
         \x20 c = cell(W { w_n: 5, w_tag: \"x\" }, -1);\n\
         \x20 assert(c.h_q == 5, \"the library reached its own private helper: {c.h_q}\");\n\
         \x20 e = cell(W { w_n: 0, w_tag: \"x\" }, -1);\n\
         \x20 assert(e.h_q == 0, \"and the helper answered false: {e.h_q}\");\n\
         }\n",
    )?;
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    for extra in [vec![], vec!["--native"]] {
        let out = run_lib_test_in_temp_cwd(loft_bin, &pkg, "probe", &extra)?;
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            combined.contains("test result: ok")
                && !combined.contains("native compile:")
                && !combined.contains("test result: FAILED"),
            "loft test {extra:?} on a package whose test file shadows a library fn name:\n{combined}"
        );
    }
    let _ = fa::remove_dir_all(&root);
    Ok(())
}

/// The native counterpart of `wrap.rs::library_suite`: every `lib/<pkg>/tests/*.loft` under
/// `--native`, skipping the packages and files `LIB_*_NATIVE_SKIP` lists.  Shells out
/// `cd lib/<pkg> && loft --native test <stem>` so it reuses the CLI's package resolution and
/// library linkage.  Holds `native_suite_lock` so it serialises with the other native suites.
// @speed 2.5
#[test]
fn native_library_suite() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    let mut files: Vec<PathBuf> = Vec::new();
    for pkg in fa::read_dir("lib")? {
        // Skip the `.loft_test_tmp_*` artifact-isolation dirs (see
        // run_lib_test_in_temp_cwd) so they're never discovered as packages.
        if pkg.file_name().is_some_and(|n| n.starts_with('.')) {
            continue;
        }
        let tests_dir = pkg.os_spelling().join("tests");
        if !fa::is_dir(&tests_dir) {
            continue;
        }
        for f in fa::read_dir(&tests_dir)? {
            let p = f.os_spelling();
            if fa::extension(&p).is_some_and(|e| e.eq_ignore_ascii_case("loft")) {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut failures: Vec<String> = Vec::new();
    let mut ran = 0;
    for entry in files {
        if native_lib_test_skipped(&entry) {
            println!("skip {entry:?} (LIB_*_NATIVE_SKIP — @P321)");
            continue;
        }
        let pkg_dir = entry.parent().and_then(|d| d.parent()).unwrap_or(&entry);
        let stem = fa::file_stem(&entry).unwrap_or_default();
        println!("native lib test {entry:?}");
        let out = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, &stem, &["--native"])?;
        ran += 1;
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        // `loft test` exits 0 even on a caught crash; detect failure by markers.
        let failed = !out.status.success()
            || combined.contains("SIGSEGV")
            || combined.contains("panicked")
            || combined.contains("native compile:")
            || combined.contains("test result: FAILED")
            || !combined.contains("test result: ok");
        if failed {
            let tail: Vec<&str> = combined.lines().rev().take(5).collect();
            failures.push(format!("{entry:?}: {}", tail.join(" | ")));
        }
    }
    if !failures.is_empty() {
        return Err(Error::other(format!(
            "{} of {ran} native library tests failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        )));
    }
    println!("native_library_suite: {ran} native library tests passed");
    Ok(())
}

/// A package's native part, called from a program on every backend: the `[c] shim` of
/// `c_scalar_pkg` is linked into the program by `--native` and called by the interpreter
/// through the same C.  The package's test asserts the oracles 42 and -1 (a narrowed return
/// would bring -1 back as 4294967295), so a broken link or a wrong value fails loudly — not
/// vacuously.  A package's own Rust native library is refused (@PLN225,
/// `tests/native_loader.rs`).
#[test]
fn native_crate_package_links_and_runs_via_cabi() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    let cells: [(&str, &[&str]); 2] = [
        ("tests/lib/c_scalar_pkg", &["--native"]),
        ("tests/lib/c_scalar_pkg", &["--interpret"]),
    ];
    for (pkg, mode) in cells {
        let out = run_lib_test_in_temp_cwd(loft_bin, Path::new(pkg), "answer", mode)?;
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !combined.contains("no implementation"),
            "{pkg} {mode:?}: the native symbol was not linked:\n{combined}"
        );
        assert!(
            combined.contains("test result: ok"),
            "{pkg} {mode:?}: the package's test did not pass:\n{combined}"
        );
    }
    Ok(())
}

/// A program that says `#link static` ships as ONE file: its package's native C is linked
/// into the binary, which runs alone in an empty directory.  The control is the same program
/// under `#link dynamic`, whose `--out` writes the package's library beside the binary and
/// whose binary, alone, refuses naming the package — so the run alone can fail.  The fixture's
/// two answers are its ABI oracle: 42, and -1, which a narrowed return would bring back as
/// 4294967295.
#[test]
fn a_static_link_ships_one_file_that_runs_alone() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let lib_dir = std::env::current_dir()?.join("tests/lib");
    let library_prefix = format!("{}c_scalar_pkg_shim_", std::env::consts::DLL_PREFIX);
    let root = std::env::temp_dir().join(format!("loft_static_link_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    for (mode, ships_a_library) in [("static", false), ("dynamic", true)] {
        let dir = root.join(mode);
        fa::create_dir_all(&dir)?;
        let program = dir.join("p.loft");
        fa::write(
            &program,
            format!(
                "#link {mode}\nuse c_scalar_pkg;\nfn main() {{ println(\"{{c_scalar_pkg::native_answer()}} {{c_scalar_pkg::native_sentinel()}}\"); }}\n"
            ),
        )?;
        let dist = dir.join("dist");
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .args(["--native", "--lib"])
            .arg(&lib_dir)
            .arg("--out")
            .arg(&dist)
            .arg(&program)
            .env("LOFT_TIMEOUT", "600")
            .output()?;
        assert!(
            out.status.success(),
            "[#link {mode}] --out failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let exe_name = format!("p{}", std::env::consts::EXE_SUFFIX);
        let libraries = fa::read_dir(&dist)?
            .iter()
            .filter_map(|e| e.os_name())
            .filter(|n| n.to_string_lossy().starts_with(&library_prefix))
            .count();
        assert_eq!(
            libraries > 0,
            ships_a_library,
            "[#link {mode}] --out {} the package's library",
            if libraries > 0 {
                "wrote"
            } else {
                "did not write"
            }
        );
        let alone = dir.join("alone");
        fa::create_dir_all(&alone)?;
        fa::copy(dist.join(&exe_name), alone.join(&exe_name))?;
        let run = loft::platform::process::harness_command(alone.join(&exe_name))
            .current_dir(&alone)
            .env("LOFT_HOME", &alone)
            .env_remove("LOFT_LIB_PATH")
            .output()?;
        let stdout = String::from_utf8_lossy(&run.stdout);
        let stderr = String::from_utf8_lossy(&run.stderr);
        if ships_a_library {
            assert!(
                !run.status.success() && stderr.contains("`c_scalar_pkg` 0.1.0"),
                "[#link dynamic] alone, the binary must refuse naming its package:\n{stdout}{stderr}"
            );
        } else {
            assert_eq!(
                stdout.trim(),
                "42 -1",
                "the static binary, alone in a directory: {stderr}"
            );
        }
    }
    let _ = fa::remove_dir_all(&root);
    Ok(())
}

/// @PLN195 — a dynamic native program finds its libraries from a search order loft owns,
/// not from a path the building machine wrote into it.  `--out` writes the program and its
/// library side by side; copied to an empty directory with `LOFT_HOME` pointing at nothing,
/// it runs.  The cells after it move the one library file and must each still answer — a
/// shared root's `<package>-<version>/`, `LOFT_LIB_PATH` — and the controls take it away: alone, and
/// beside a DIFFERENT version, the run must fail naming the package, the version and the
/// directories searched.  The binary must not name the directory its library was built in.
#[test]
fn a_dynamic_program_finds_its_libraries_where_loft_looks() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let lib_dir = std::env::current_dir()?.join("tests/lib");
    let root = std::env::temp_dir().join(format!("loft_lib_search_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let src = root.join("src");
    fa::create_dir_all(&src)?;
    let program = src.join("p.loft");
    fa::write(
        &program,
        "use c_scalar_pkg;\nfn main(args: vector<text>) { println(\"{c_scalar_pkg::native_answer()} {c_scalar_pkg::native_sentinel()} {len(args)}\"); }\n",
    )?;
    let dist = root.join("dist");
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .args(["--ship", "--native", "--dynamic", "--lib"])
        .arg(&lib_dir)
        .arg("--out")
        .arg(&dist)
        .arg(&program)
        .env("LOFT_TIMEOUT", "600")
        .output()?;
    assert!(
        out.status.success(),
        "--out failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let exe_name = format!("p{}", std::env::consts::EXE_SUFFIX);
    let library_prefix = format!("{}c_scalar_pkg_shim_", std::env::consts::DLL_PREFIX);
    let library_file = fa::read_dir(&dist)?
        .iter()
        .filter_map(|e| e.os_name())
        .map(|n| n.to_string_lossy().into_owned())
        .find(|n| n.starts_with(&library_prefix))
        .unwrap_or_else(|| panic!("--out wrote no `{library_prefix}*` library"));
    let built_lib = dist.join(&library_file);
    // The directory the library was built from — the path a linked binary would name as an
    // rpath.
    let build_dir = lib_dir.join("c_scalar_pkg");
    let needle = build_dir.to_string_lossy().into_owned();
    let bytes = fa::read(dist.join(&exe_name))?;
    assert!(
        !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
        "the binary names the build directory `{needle}`"
    );

    let empty_home = root.join("home");
    fa::create_dir_all(&empty_home)?;
    let run = |dir: &Path, home: &Path, args: &[String], env: Option<&Path>| {
        let mut cmd = loft::platform::process::harness_command(dir.join(&exe_name));
        cmd.current_dir(dir)
            .args(args)
            .env("LOFT_HOME", home)
            .env_remove("LOFT_LIB_PATH");
        if let Some(e) = env {
            cmd.env("LOFT_LIB_PATH", e);
        }
        cmd.output()
    };
    let place = |name: &str| -> std::io::Result<PathBuf> {
        let d = root.join(name);
        fa::create_dir_all(&d)?;
        fa::copy(dist.join(&exe_name), d.join(&exe_name))?;
        Ok(d)
    };
    let expect_ok = |cell: &str, o: std::process::Output| {
        assert_eq!(
            String::from_utf8_lossy(&o.stdout).trim(),
            "42 -1 0",
            "[{cell}] {}",
            String::from_utf8_lossy(&o.stderr)
        );
    };

    // Beside the binary: the copied dist directory.
    let moved = place("moved")?;
    fa::copy(&built_lib, moved.join(&library_file))?;
    expect_ok("beside", run(&moved, &empty_home, &[], None)?);

    // A shared root, by version.
    let shared = root.join("shared");
    let versioned = shared.join(".loft").join("lib").join("c_scalar_pkg-0.1.0");
    fa::create_dir_all(&versioned)?;
    fa::copy(&built_lib, versioned.join(&library_file))?;
    let alone = place("alone")?;
    expect_ok("~/.loft/lib/<pkg>-<ver>", run(&alone, &shared, &[], None)?);

    // The variable — and an argument the program is given stays the program's.
    expect_ok("LOFT_LIB_PATH", run(&alone, &empty_home, &[], Some(&dist))?);
    let arg = run(
        &alone,
        &empty_home,
        &["--loft-lib-path=x".to_string()],
        Some(&dist),
    )?;
    assert_eq!(
        String::from_utf8_lossy(&arg.stdout).trim(),
        "42 -1 1",
        "an argument was taken from the program: {}",
        String::from_utf8_lossy(&arg.stderr)
    );

    // Controls: nowhere, and only another version — each a loft error that names it all.
    let other = root.join("other");
    let wrong = other.join(".loft").join("lib").join("c_scalar_pkg-0.0.9");
    fa::create_dir_all(&wrong)?;
    fa::copy(&built_lib, wrong.join(&library_file))?;
    for (cell, home) in [("nowhere", &empty_home), ("another version", &other)] {
        let o = run(&alone, home, &[], None)?;
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "[{cell}] exit: {err}");
        assert!(
            err.contains("`c_scalar_pkg` 0.1.0") && err.contains("searched:"),
            "[{cell}] the error names neither package nor version:\n{err}"
        );
        if cell == "another version" {
            assert!(
                err.contains("c_scalar_pkg-0.0.9"),
                "[{cell}] the other version is not named:\n{err}"
            );
        }
    }
    let _ = fa::remove_dir_all(&root);
    Ok(())
}

/// @PLN197 — a C library a package ships travels with the program, and never by a path of
/// the machine that built it.  The `c_abi` fixtures carry both kinds: `lcshim`'s `[c] shim`,
/// which loft compiles, and `lcabi`'s prebuilt `liblc_types`.  Built with `--out`, both
/// ways: the dynamic program finds both libraries beside itself through loft's search order;
/// the static one has the shim's code INSIDE it (on Windows compiled by MSVC `cl`, @PLN197 C)
/// and finds `liblc_types`, which has no archive, the same way.  Neither binary may name the
/// directory the packages were built in — the copy is renamed away before the moved runs,
/// which must answer exactly as `--interpret` does.  Controls: the dynamic program without
/// the shim beside it, and the static one without `liblc_types`, must each stop naming the
/// package — so a binary that still reached into the build tree cannot pass.
#[test]
fn a_shipped_c_library_travels_with_the_program() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        println!("a_shipped_c_library_travels_with_the_program: skipped (no cc)");
        return Ok(());
    }
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi");
    let built = loft::platform::process::harness_command("make")
        .arg("-C")
        .arg(&fixture)
        .output()?;
    assert!(
        built.status.success(),
        "the fixture library must build: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
        fa::create_dir_all(to)?;
        for entry in fa::read_dir(from)? {
            let name = entry.os_name().expect("fixture entry");
            let (src, dst) = (from.join(&name), to.join(&name));
            if fa::symlink_metadata(&src)?.file_type().is_dir() {
                copy_tree(&src, &dst)?;
            } else {
                fa::copy(&src, &dst)?;
            }
        }
        Ok(())
    }
    let root = std::env::temp_dir().join(format!("loft_shipped_c_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let tree = root.join("tree");
    copy_tree(&fixture, &tree)?;
    // The shim is built fresh, from the copy, so its artefacts belong to this run.
    let _ = fa::remove_dir_all(tree.join("pkg").join("lcshim").join("native-auto"));
    let program = root.join("p.loft");
    fa::write(
        &program,
        "use lcshim::*;\nuse lcabi::*;\nfn main() {\n  println(\"mod {shim_mod(17, 5)} owned {shim_owned(7)} neg {lc_neg_i32(5)}\");\n}\n",
    )?;
    let loft_cmd = |args: &[&str]| -> std::io::Result<std::process::Output> {
        loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .args(args)
            .arg("--lib")
            .arg(tree.join("pkg"))
            .arg(&program)
            .env("LOFT_TIMEOUT", "600")
            .output()
    };
    let interp = loft_cmd(&["--interpret"])?;
    let want = String::from_utf8_lossy(&interp.stdout).trim().to_string();
    assert_eq!(want, "mod 2 owned shim-7 neg -5", "the oracle itself");

    let exe = format!("p{}", std::env::consts::EXE_SUFFIX);
    // The shim goes INTO a static binary wherever the host has a shim archiver — everywhere
    // but a Windows target other than MSVC.  An MSVC host without one is a failure, not a
    // skip: it would leave the static `.exe` needing the shim DLL beside it.
    let archives_link = loft::platform::shim_archiver().is_some();
    let triple = loft::cache::host_triple();
    assert!(
        archives_link || triple.contains("windows-gnu"),
        "no shim archiver on {triple} — a static program would keep its shim beside it"
    );
    let needle = tree.to_string_lossy().into_owned();
    // `shipped` names no link mode: `--ship` links the libraries in by default
    // (`native_utils::link_static_unless_named`), so it is the static program.
    for (mode, flag) in [
        ("dynamic", Some("--dynamic")),
        ("static", Some("--static")),
        ("shipped", None),
    ] {
        let dist = root.join(mode);
        let dist_s = dist.to_string_lossy().into_owned();
        let mut args = vec!["--ship"];
        args.extend(flag);
        args.extend(["--out", dist_s.as_str()]);
        let out = loft_cmd(&args)?;
        assert!(
            out.status.success(),
            "[{mode}] --out failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let bytes = fa::read(dist.join(&exe))?;
        assert!(
            !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
            "[{mode}] the binary names the build tree `{needle}`"
        );
        let shim_beside = fa::read_dir(&dist)?
            .into_iter()
            .filter_map(|e| e.os_name())
            .any(|n| n.to_string_lossy().contains("lcshim_shim_"));
        assert_eq!(
            shim_beside,
            mode == "dynamic" || !archives_link,
            "[{mode}] the shim is {} the program",
            if shim_beside { "beside" } else { "not beside" }
        );
    }
    let hidden = root.join("tree-hidden");
    fa::rename(&tree, &hidden)?;
    let run = |dir: &Path| {
        loft::platform::process::harness_command(dir.join(&exe))
            .current_dir(dir)
            .env("LOFT_HOME", root.join("no-home"))
            .env_remove("LOFT_LIB_PATH")
            .output()
    };
    for mode in ["dynamic", "static", "shipped"] {
        let o = run(&root.join(mode))?;
        assert_eq!(
            String::from_utf8_lossy(&o.stdout).trim(),
            want,
            "[{mode}, build tree gone] {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
    // Controls: take away a library each program needs at run time.  `liblc_types` is found
    // by stem, not spelled from `DLL_PREFIX`: the MinGW build keeps the `lib` prefix that a
    // Windows DLL name otherwise lacks.
    let lc_types = fa::read_dir(root.join("static"))?
        .into_iter()
        .filter_map(|e| e.os_name())
        .find(|n| {
            let n = n.to_string_lossy();
            n.contains("lc_types") && n.ends_with(std::env::consts::DLL_SUFFIX)
        })
        .expect("the static dist holds liblc_types");
    let shim = fa::read_dir(root.join("dynamic"))?
        .into_iter()
        .filter_map(|e| e.os_name())
        .find(|n| n.to_string_lossy().contains("lcshim_shim_"))
        .expect("the dynamic dist holds the shim");
    fa::remove_file(root.join("dynamic").join(&shim))?;
    fa::remove_file(root.join("static").join(&lc_types))?;
    for (mode, package) in [("dynamic", "`lcshim`"), ("static", "`lcabi`")] {
        let o = run(&root.join(mode))?;
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "[{mode} control] exit: {err}");
        assert!(
            err.contains(package) && err.contains("searched:"),
            "[{mode} control] the error does not name {package}:\n{err}"
        );
    }
    fa::rename(&hidden, &tree)?;
    let _ = fa::remove_dir_all(&root);
    Ok(())
}

/// loft#1311 — a FN-LEVEL `@EXPECT_FAIL` excuses one function, not the whole file.
///
/// The documented `// @EXPECT_FAIL: <reason>` form under a declaration names that one
/// function: the script corpus must not drop the file for it, and `loft --tests <file>
/// --native` must skip the excused function while every sibling still runs and passes.
/// The control is the same file with a sibling broken, which must fail — so a run that
/// skipped the whole file cannot pass.
#[test]
fn a_fn_level_expect_fail_keeps_its_siblings_in_the_native_suite() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());

    let src = "\
fn test_1311_sibling_before() { assert(1 == 1, \"before\"); }

// @EXPECT_FAIL: deliberately broken
fn test_1311_excused() { assert(false, \"deliberate\"); }

fn test_1311_sibling_after() { assert(2 == 2, \"after\"); }
";

    // Parsed alone, the tag names one function and declares nothing file-level.
    let (fns, file_level) = common::expect_fail_fns(src);
    assert!(
        fns.contains("test_1311_excused"),
        "the documented `@EXPECT_FAIL: <reason>` form must name its fn; got {fns:?}"
    );
    assert!(
        !file_level,
        "a tag under a declaration is fn-level, not file-level"
    );
    // Control: the SAME tag in the header IS file-level, so the drop still has a subject.
    let (_, header_level) = common::expect_fail_fns("// @EXPECT_FAIL: whole file\nfn t() { }\n");
    assert!(header_level, "a header tag must still read as file-level");

    let dir = std::env::temp_dir().join(format!("loft1311_{}", std::process::id()));
    fa::create_dir_all(&dir)?;
    let run = |name: &str, body: &str| -> std::io::Result<(bool, String)> {
        let entry = dir.join(name);
        fa::write(&entry, body)?;
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg("--tests")
            .arg(&entry)
            .arg("--native")
            .env("LOFT_TIMEOUT", "300")
            .output()?;
        Ok((
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        ))
    };
    let (ok, report) = run("fn_level.loft", src)?;
    assert!(
        ok && report.contains("2 passed") && report.contains("1 skipped"),
        "the excused fn must be skipped and both siblings must run and pass:\n{report}"
    );
    let (ok, report) = run(
        "fn_level_broken.loft",
        &src.replace("assert(2 == 2", "assert(2 == 3"),
    )?;
    assert!(
        !ok && report.contains("test_1311_sibling_after"),
        "a broken sibling must fail the run, naming it:\n{report}"
    );
    let _ = fa::remove_dir_all(&dir);
    Ok(())
}

/// A package's `[c] shim` on the wasm target: loft compiles the shim's C into the module
/// beside the program's own C, and the bindings are called directly at wasm32's widths.  The
/// fixture's four crossings — a scalar, a `char *` return, a vector in by pointer and count,
/// a vector of records as bytes — answer under `wasmtime` what the interpreter answers.
/// Skips without a wasm32 C toolchain or `wasmtime`.
#[test]
#[ignore = "a wasm32 build of a whole program — runs nightly in miri.yml's `release-gate-sweeps` job"]
fn a_c_shim_links_into_the_wasm_target() -> std::io::Result<()> {
    let have = |tool: &str| {
        loft::platform::process::harness_command(tool)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if loft::platform::wasm_c_toolchain().triple.is_none() || !have("wasmtime") {
        eprintln!("skipped: needs a wasm32 C toolchain (clang and a WASI sysroot) and wasmtime");
        return Ok(());
    }
    let libdir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi/pkg");
    let dir = std::env::temp_dir().join(format!("loft-shim-wasm-{}", std::process::id()));
    fa::create_dir_all(&dir)?;
    let src = "use lcshim::*;\n\
fn main() {\n\
  v = [10, 20, 30];\n\
  out: vector<float> = [0.0, 0.0, 0.0];\n\
  shim_fill(out, 1500);\n\
  px = [Px { r: 1, g: 2, b: 3 }, Px { r: 0, g: 0, b: 255 }];\n\
  println(\"{shim_mod(17, 5)} {wrapped_owned(7)} {shim_sum(v)} {out} {shim_rgb_sum(px)}\");\n\
}\n";
    // 17 % 5; shim-7 bracketed; 10 + 40 + 90; 1.5, 1.501, 1.502; 65536 + 512 + 3 + 255.
    let want = "2 <shim-7> 140 [1.5,1.501,1.502] 66306\n";
    let file = dir.join("w.loft");
    fa::write(&file, src)?;
    let loft = env!("CARGO_BIN_EXE_loft");
    let interp = loft::platform::process::harness_command(loft)
        .arg("--interpret")
        .arg("--lib")
        .arg(&libdir)
        .arg(&file)
        .output()?;
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout),
        want,
        "the interpreter: {}",
        String::from_utf8_lossy(&interp.stderr)
    );
    let wasm = dir.join("w.wasm");
    let build = loft::platform::process::harness_command(loft)
        .env("LOFT_TIMEOUT", "900")
        .arg("--native")
        .arg("--native-wasm")
        .arg(&wasm)
        .arg("--lib")
        .arg(&libdir)
        .arg(&file)
        .output()?;
    assert!(
        build.status.success(),
        "the wasm build with the shim linked in: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = loft::platform::process::harness_command("wasmtime")
        .args(["-W", "exceptions=y"])
        .arg(&wasm)
        .output()?;
    assert!(
        run.status.success(),
        "wasmtime: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        want,
        "the wasm program"
    );
    let _ = fa::remove_dir_all(&dir);
    Ok(())
}

/// Two packages with a `[c] shim` link into one wasm program (loft#2280): neither package's
/// C may bring a symbol the other, or the program, also defines (`duplicate symbol`).
/// `lcshim2` needs `strlen`, `lcshim` brings its own copy.  Skips without a wasm32 C
/// toolchain or `wasmtime`.
#[test]
#[ignore = "a wasm32 build of a whole program — runs nightly in miri.yml's `release-gate-sweeps` job"]
fn two_c_shims_link_into_the_wasm_target() -> std::io::Result<()> {
    let have = |tool: &str| {
        loft::platform::process::harness_command(tool)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if loft::platform::wasm_c_toolchain().triple.is_none() || !have("wasmtime") {
        eprintln!("skipped: needs a wasm32 C toolchain (clang and a WASI sysroot) and wasmtime");
        return Ok(());
    }
    let libdir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi/pkg");
    let dir = std::env::temp_dir().join(format!("loft-two-shims-{}", std::process::id()));
    fa::create_dir_all(&dir)?;
    let src = "use lcshim::*;\nuse lcshim2::*;\n\
fn main() {\n\
  println(\"{shim_mod(17, 5)} {shim2_len(\"wasm\")}\");\n\
}\n";
    // 17 % 5; 10 times the length of "wasm".
    let want = "2 40\n";
    let file = dir.join("two.loft");
    fa::write(&file, src)?;
    let loft = env!("CARGO_BIN_EXE_loft");
    let wasm = dir.join("two.wasm");
    let build = loft::platform::process::harness_command(loft)
        .env("LOFT_TIMEOUT", "900")
        .arg("--native")
        .arg("--native-wasm")
        .arg(&wasm)
        .arg("--lib")
        .arg(&libdir)
        .arg(&file)
        .output()?;
    assert!(
        build.status.success(),
        "the wasm build with two shims linked in: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = loft::platform::process::harness_command("wasmtime")
        .args(["-W", "exceptions=y"])
        .arg(&wasm)
        .output()?;
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        want,
        "the wasm program: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let _ = fa::remove_dir_all(&dir);
    Ok(())
}
