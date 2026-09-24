// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

extern crate loft;

use loft::compile::byte_code;
#[cfg(debug_assertions)]
use loft::compile::show_code;
use loft::data::Data;
use loft::generation::Output;
#[cfg(debug_assertions)]
use loft::log_config::LogConfig;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;
use std::collections::HashSet;
#[cfg(debug_assertions)]
use std::fs::File;
use std::io::Error;
#[cfg(debug_assertions)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
mod common;
use common::cached_default;

/// Process-wide lock: prevents any two `wrap` tests from running concurrently.
///
/// Several scripts in `tests/scripts/` are not safe to execute in parallel with
/// themselves or each other — for example `11-files.loft` creates and deletes real
/// files, and `loft_suite` already runs that same file.  Cargo's default test runner
/// would execute `loft_suite` and `files()` (or any other pair) simultaneously,
/// causing races and spurious failures.
///
/// Every public `#[test]` in this file acquires the lock before calling `run_test`,
/// so all wrap tests are serialised within the process.  Cross-process parallelism
/// (e.g. two `cargo test` invocations at once) is the caller's responsibility.
static WRAP_LOCK: Mutex<()> = Mutex::new(());

/// Files in `tests/docs/` that `dir` must not run.
///
/// Empty, and staying empty is the point: every page the site publishes is a
/// program that runs, so an entry here is a page whose code a reader cannot
/// trust.  Add one only with the open issue that explains it, and delete it the
/// day that issue closes.
///
/// It held `14-image` and `21-random` for years, because a doc page that
/// `use`s a library cannot be built by this embedded harness — which is the
/// wrong place to solve that: a library's guide belongs in the library, where
/// its own CI runs it.  Both moved out (@PLN149 step 9), and the list emptied.
const SUITE_SKIP: &[&str] = &[];

/// Docs files skipped in `--native-wasm` mode.  An entry carries the open issue
/// that explains it and the condition that removes it (TESTING.md § Every skip
/// says why, how it runs instead, and when it ends).
const WASM_SKIP: &[&str] = &[
    // (empty) `19-threading.loft` sat here as "the WASM threading model differs"
    // long after it compiled and ran green under wasmtime.
];

/// Compile a `.loft` file to a WebAssembly binary via the loft codegen + rustc, then
/// optionally run it with `wasmtime`.
///
/// Skips silently (returns `Ok`) if the `wasm32-wasip2` target is not installed or if
/// `rustc` is not found.  Runs the wasm with `wasmtime` if it is in PATH; otherwise
/// only verifies that compilation succeeds.
fn run_wasm_test(entry: &Path) -> std::io::Result<()> {
    let stem = entry
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .replace('-', "_");
    println!("wasm  {entry:?}");

    // Parse
    let source = std::fs::read_to_string(entry)?;
    let expected = expected_warnings(&source);
    let (exp_errors, exp_ann_warns) = expected_annotations(&source);
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    // Honour `// @ARGS: --lib <dir>` — same logic as `run_test`.
    for line in source.lines().take(20) {
        if let Some(args) = line.trim().strip_prefix("// @ARGS:") {
            let mut tokens = args.split_whitespace();
            while let Some(tok) = tokens.next() {
                if tok == "--lib"
                    && let Some(dir) = tokens.next()
                {
                    p.lib_dirs.push(dir.to_string());
                }
            }
        }
    }
    let start_def = p.data.definitions();
    p.parse(&entry.to_string_lossy(), false);
    for l in p.diagnostics.lines() {
        println!("{l}");
    }
    // Run the check whenever the file DECLARES something, not only when the parser said
    // something.  Gating on `!p.diagnostics.is_empty()` was the second way an expectation
    // went inert: a file whose diagnostics disappeared entirely never reached
    // `check_diagnostics`, so its `@EXPECT_ERROR` lines were not merely unmatched — they
    // were never looked at (loft#929).
    if !p.diagnostics.is_empty()
        || !expected.is_empty()
        || !exp_errors.is_empty()
        || !exp_ann_warns.is_empty()
    {
        check_diagnostics(
            &p.diagnostics.lines(),
            &expected,
            &exp_errors,
            &exp_ann_warns,
            true,
        )?;
    }
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let end_def = p.data.definitions();
    let main_nr = p.data.def_nr("n_main");
    let entry_defs: Vec<u32> = if main_nr < end_def {
        vec![main_nr]
    } else {
        (start_def..end_def).collect()
    };

    // Generate Rust source
    let tmp_rs = std::env::temp_dir().join(format!("loft_wasm_{stem}.rs"));
    {
        let mut f = std::fs::File::create(&tmp_rs)?;
        let mut out = Output::new(&p.data, &state.database);
        out.output_native_reachable(&mut f, start_def, end_def, &entry_defs)?;
    }

    // Compile for wasm32-wasip2
    let tmp_wasm = std::env::temp_dir().join(format!("loft_wasm_{stem}.wasm"));
    let mut cmd = std::process::Command::new("rustc");
    cmd.arg("--edition=2024")
        .arg("--target")
        .arg("wasm32-wasip2")
        .arg("--crate-type")
        .arg("bin")
        .arg("-O")
        .arg("-o")
        .arg(&tmp_wasm)
        .arg(&tmp_rs);
    // Look for a wasm32-wasip2 loft rlib next to the test binary's deps
    // (only present if the user ran `cargo build --target wasm32-wasip2` first).
    let wasm_rlib = std::env::current_exe().ok().and_then(|exe| {
        // Walk up from target/debug/deps to target/, then into wasm32-wasip2/debug/
        let target_dir = exe.parent()?.parent()?.parent()?;
        let rlib_dir = target_dir.join("wasm32-wasip2").join("debug");
        std::fs::read_dir(&rlib_dir)
            .ok()?
            .filter_map(|e| e.ok())
            .find(|e| {
                let n = e.file_name();
                let s = n.to_string_lossy();
                s.starts_with("libloft") && s.ends_with(".rlib")
            })
            .map(|e| (e.path(), rlib_dir))
    });
    if let Some((rlib, deps_dir)) = wasm_rlib {
        cmd.arg("--extern")
            .arg(format!("loft={}", rlib.display()))
            .arg("-L")
            .arg(&deps_dir);
    }
    let compile_out = cmd.output();
    let _ = std::fs::remove_file(&tmp_rs);
    let compile_out = match compile_out {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("  rustc not found — skipping wasm test for {stem}");
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    if !compile_out.status.success() {
        let stderr = String::from_utf8_lossy(&compile_out.stderr);
        // wasm32-wasip2 target not installed → skip gracefully
        if stderr.contains("target may not be installed") || stderr.contains("can't find crate") {
            println!("  wasm32-wasip2 target or loft wasm rlib not available — skipping {stem}");
            let _ = std::fs::remove_file(&tmp_wasm);
            return Ok(());
        }
        eprintln!("rustc (wasm) failed for {stem}:\n{stderr}");
        let _ = std::fs::remove_file(&tmp_wasm);
        return Err(Error::from(std::io::ErrorKind::Other));
    }

    // Run with wasmtime if available
    match std::process::Command::new("wasmtime")
        .arg(&tmp_wasm)
        .status()
    {
        Ok(s) => {
            let _ = std::fs::remove_file(&tmp_wasm);
            if !s.success() {
                eprintln!("wasmtime failed for {stem} (exit {:?})", s.code());
                return Err(Error::from(std::io::ErrorKind::Other));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("  wasmtime not found — compiled ok, skipping run for {stem}");
            let _ = std::fs::remove_file(&tmp_wasm);
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_wasm);
            return Err(e);
        }
    }
    Ok(())
}

/// Run every `.loft` file in `tests/docs/` in alphabetical order, skipping
/// files listed in `SUITE_SKIP` (known broken; tracked as open issues).
/// These files also serve as user-facing documentation (HTML generation via `@NAME`/`@TITLE`).
#[test]
fn dir() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut files: Vec<PathBuf> = std::fs::read_dir("tests/docs")?
        .filter_map(|f| f.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .collect();
    files.sort();
    for entry in files {
        let name = entry.file_name().unwrap_or_default().to_string_lossy();
        if SUITE_SKIP.iter().any(|s| *s == name.as_ref()) {
            println!("skip {entry:?} (known issue — see SUITE_SKIP)");
            continue;
        }
        run_test(entry, false, true)?;
    }
    Ok(())
}

/// Run every `.loft` file in `tests/comparisons/` — the language-comparison claims, as
/// programs.
///
/// These back `doc/00-vs-rust.html` and `doc/00-vs-python.html`, which are the ONLY reference
/// pages with no `tests/docs/` source: they are hand-written HTML, and until 2026-09-23 the 28
/// loft samples on them were the only published code nothing executed.  A file here asserts one
/// claim those pages make, so a claim that stops being true turns this test red instead of
/// leaving a stale sentence on the page a newcomer reads first.
///
/// Same `run_test` as `dir()` and `loft_suite()`, so the entry-point rule and the
/// `@EXPECT_WARNING` / `@EXPECT_ERROR` annotations behave identically.  There is deliberately
/// no skip list: a comparison claim that cannot run is a claim to delete, not to except.
///
/// The contract each file follows is `tests/comparisons/README.md`; the subject index and the
/// links to every rationale are `doc/claude/SUBJECTS.md`.
#[test]
fn comparisons() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut files: Vec<PathBuf> = std::fs::read_dir("tests/comparisons")?
        .filter_map(|f| f.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "tests/comparisons/ is empty — the directory exists to gate the comparison claims, \
         and an empty one passes while proving nothing"
    );
    for entry in files {
        run_test(entry, false, true)?;
    }
    Ok(())
}

/// Compile every `.loft` file in `tests/docs/` to WebAssembly (wasm32-wasip2),
/// skipping files listed in `WASM_SKIP`.
///
/// Skips silently if `rustc` is not in PATH or the `wasm32-wasip2` target is not
/// installed.  Runs the resulting `.wasm` with `wasmtime` if it is in PATH; otherwise
/// only verifies compilation succeeds.
// @speed 1.2
#[test]
fn wasm_dir() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut files: Vec<PathBuf> = std::fs::read_dir("tests/docs")?
        .filter_map(|f| f.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .collect();
    files.sort();
    for entry in files {
        let name = entry.file_name().unwrap_or_default().to_string_lossy();
        if WASM_SKIP.iter().any(|s| *s == name.as_ref()) {
            println!("skip {entry:?} (wasm skip list — see WASM_SKIP)");
            continue;
        }
        run_wasm_test(&entry)?;
    }
    Ok(())
}

/// Run the `.loft` files in `tests/scripts/` in alphabetical order — all of them, or one CHUNK.
/// These are standalone loft programs that exercise compiler and interpreter features.
/// Scripts may use `fn main()` or `fn test_*()` entry points — `run_test`
/// discovers and executes all zero-parameter user functions automatically.
/// To run a single file use the individual test functions below, e.g.:
///   cargo test --test wrap integers
///
/// The per-commit run is CHUNKED (`loft_suite_00` … `loft_suite_07`, every
/// [`LOFT_SUITE_CHUNKS`]-th script from each offset): one test over the whole corpus took
/// 650-760 s on the Windows runner, and the duration gate allows a test half its 600 s limit.
/// What a chunk gives up is the WHOLE-corpus interaction — one script corrupting shared state
/// that a later, innocent script dies of (how loft#920 was found) — so that run is kept as
/// [`loft_suite_whole_corpus`], `#[ignore]`d and run by the nightly release-gate job.
fn loft_suite_run(chunk: Option<usize>) -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut files: Vec<PathBuf> = std::fs::read_dir("tests/scripts")?
        .filter_map(|f| f.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .collect();
    files.sort();
    // `LOFT_SCRIPT_FIRST` / `LOFT_SCRIPT_LAST` — run only a WINDOW of the sorted corpus.
    //
    // The instrument for a fault that only appears in the whole-corpus run: one script
    // corrupts the store table and a LATER, innocent one dies of it, so the question is
    // "which earlier script?" and the only way to ask it is to run a subset. Without this
    // the answer had to be guessed — loft#920 was attributed to the wrong script twice.
    //
    // A window rather than a limit, because both ends move: shrink `LAST` to find the
    // script that DIES, then raise `FIRST` to find the earliest one whose presence still
    // kills it. Indices are into the sorted list and are printed on every run, so a bisect
    // step is a number rather than a filename. Pin the order with
    // `LOFT_HASH_SEED=0x0123456789abcdef` — allocation order is what decides where latent
    // corruption surfaces.
    let idx_env = |k: &str, dflt: usize| -> usize {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(dflt)
    };
    let first = idx_env("LOFT_SCRIPT_FIRST", 0);
    let last = idx_env("LOFT_SCRIPT_LAST", files.len());
    if first != 0 || last != files.len() {
        eprintln!(
            "loft_suite: window [{first}, {last}) of {} scripts — NOT a full run",
            files.len()
        );
        files = files
            .into_iter()
            .enumerate()
            .filter(|(i, _)| *i >= first && *i < last)
            .map(|(_, p)| p)
            .collect();
    }
    if let Some(k) = chunk {
        files = files
            .into_iter()
            .enumerate()
            .filter(|(i, _)| i % LOFT_SUITE_CHUNKS == k)
            .map(|(_, p)| p)
            .collect();
    }
    // Scripts with dedicated #[ignore] wrappers are skipped here to keep
    // loft_suite green while the feature is under development.
    let skip: HashSet<&str> = ignored_scripts();
    for entry in files {
        let name = entry
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if skip.contains(name.as_str()) {
            println!("skip {entry:?} (has dedicated #[ignore] test)");
            continue;
        }
        // One row per corpus PROGRAM (`LOFT_TEST_TIMING=<file>`), because to nextest this
        // whole loop is a single test: ~895 programs with no individual time and one shared
        // output bucket.  See `common::timing` for why the row is CPU and not wall.
        let unit = common::timing::Unit::start();
        let outcome = run_test(entry, false, false);
        unit.finish(&name);
        outcome?;
    }
    Ok(())
}

/// How many tests the per-commit `loft_suite` run is split across — see [`loft_suite_run`].
const LOFT_SUITE_CHUNKS: usize = 8;

macro_rules! loft_suite_chunks {
    ($($name:ident = $k:expr),* $(,)?) => {
        $(
            #[test]
            fn $name() -> std::io::Result<()> {
                loft_suite_run(Some($k))
            }
        )*
    };
}

loft_suite_chunks!(
    loft_suite_00 = 0,
    loft_suite_01 = 1,
    loft_suite_02 = 2,
    loft_suite_03 = 3,
    loft_suite_04 = 4,
    loft_suite_05 = 5,
    loft_suite_06 = 6,
    loft_suite_07 = 7,
);

/// Every chunk index has its test: a chunk count raised without a matching test would drop that
/// slice of the corpus from every per-commit run, silently.
#[test]
fn loft_suite_chunks_cover_the_corpus() {
    let names = [
        "loft_suite_00",
        "loft_suite_01",
        "loft_suite_02",
        "loft_suite_03",
        "loft_suite_04",
        "loft_suite_05",
        "loft_suite_06",
        "loft_suite_07",
    ];
    assert_eq!(
        names.len(),
        LOFT_SUITE_CHUNKS,
        "one generated test per chunk"
    );
    let src = std::fs::read_to_string(file!()).expect("read tests/wrap.rs");
    for n in names {
        assert!(src.contains(&format!("{n} = ")), "{n} is generated");
    }
}

/// The whole corpus in ONE process, in order — the interaction coverage the chunks give up, and
/// the run `LOFT_SCRIPT_FIRST` / `LOFT_SCRIPT_LAST` bisect over.
#[test]
#[ignore = "the whole tests/scripts corpus in one process — the nightly release-gate job (miri.yml); run with --ignored"]
fn loft_suite_whole_corpus() -> std::io::Result<()> {
    loft_suite_run(None)
}

/// Every `.loft` file the two corpus runners execute, in sorted order.
///
/// `tests/docs/` and `tests/comparisons/` run through the same `run_test` as
/// `tests/scripts/`, so the entry-point rule below applies identically to all three.
fn corpus_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = ["tests/scripts", "tests/docs", "tests/comparisons"]
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .filter_map(|f| f.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .collect();
    files.sort();
    files
}

/// One top-level `fn` of a corpus file: where it starts, whether it takes parameters,
/// whether an `@EXPECT_ERROR:` annotation sits directly above it, and how many `assert`
/// calls its body contains.
struct CorpusFn {
    name: String,
    line: usize,
    zero_param: bool,
    expect_error: bool,
    asserts: usize,
}

/// Split a corpus file into its top-level functions.
///
/// A body runs from the `fn` line to the next line that is exactly `}` — the corpus's
/// house style — with a one-line `fn f() { … }` handled by looking for the closing brace
/// on the `fn` line itself.  Brace COUNTING is not usable here: `"{name}"` interpolation
/// puts unbalanced braces inside string literals.
fn corpus_fns(source: &str) -> Vec<CorpusFn> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    let mut pending_expect_error = false;
    for (i, raw) in lines.iter().enumerate() {
        if raw.trim_start().starts_with("//") {
            if common::expect_tag(raw, "@EXPECT_ERROR:").is_some() {
                pending_expect_error = true;
            }
            continue;
        }
        let Some(rest) = raw.strip_prefix("fn ") else {
            // A blank line keeps a pending annotation alive (they are often separated by
            // one); anything else at top level ends it.
            if !raw.trim().is_empty() {
                pending_expect_error = false;
            }
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        // A signature that runs past this line (`fn datetime(` + one parameter per line)
        // has no `)` here, and `None` must read as "has parameters": the reachability
        // ratchet only asks about ZERO-parameter functions, so guessing the other way
        // would invent a finding rather than miss one.
        let params = rest
            .split_once('(')
            .and_then(|(_, r)| r.split_once(')'))
            .map_or_else(|| "…".to_string(), |(p, _)| p.trim().to_string());
        let end = if raw[raw.find('{').map_or(raw.len(), |b| b + 1)..].contains('}') {
            i
        } else {
            lines
                .iter()
                .enumerate()
                .skip(i + 1)
                .find(|(_, l)| **l == "}")
                .map_or(lines.len() - 1, |(j, _)| j)
        };
        let body = lines[i..=end].join("\n");
        let asserts = body
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .map(|l| l.matches("assert(").count())
            .sum();
        out.push(CorpusFn {
            name,
            line: i + 1,
            zero_param: params.is_empty(),
            expect_error: pending_expect_error,
            asserts,
        });
        pending_expect_error = false;
    }
    out
}

/// R1 — a runtime cell may not share a file with a refusal it would never survive.
///
/// A file whose `@EXPECT_ERROR:` fires never reaches execution: `run_test` stops at
/// "errors consumed" and `native_scripts` skips the file outright.  So an `assert` in a
/// function that is NOT itself the refusal is compiled and never run, and the file passes
/// on the refusal alone.  Measured before this guard existed: 52 such assertions, twenty-one
/// of them the entire positive half of `1067-lambda-expected-type.loft` — a file whose own
/// header explains that its negative cell exists so a compiler that stopped checking could
/// not pass it, while that negative cell was what stopped the positive cells running.
///
/// The cure is the corpus's own convention: `102`/`102b`, `36`/`36b`, `1067`/`1067b` — a
/// file asserts a refusal OR runs.
///
/// An `assert` inside an `@EXPECT_ERROR` function is fine and common: it is the USE that
/// makes the refused expression matter (`assert(len(v) == 0, "unreached")`).
#[test]
fn a_refusal_file_carries_no_runtime_assertions() {
    let mut bad: Vec<String> = Vec::new();
    for entry in corpus_files() {
        let Ok(src) = std::fs::read_to_string(&entry) else {
            continue;
        };
        if !common::declares_expect_error(&src) {
            continue;
        }
        // loft#1242 — the rule now holds only where the cells cannot be ISOLATED.
        //
        // `run_test` blanks the functions an error points into and re-runs the rest, so in a
        // fn-per-cell file an assertion outside the refusing function DOES run and the `<n>b-…`
        // split is no longer needed. What it cannot do is isolate a file with a `main`: the
        // harness then runs only `main`, so a helper it calls cannot be removed without changing
        // what `main` means, and an assertion there is still compiled-and-never-run.
        //
        // Checked against `top_level_fn_spans` rather than a `fn main` grep, so this agrees with
        // the isolation itself about what a top-level function is — the two must not drift, or
        // the guard forbids exactly the files the harness now handles.
        let isolatable = top_level_fn_spans(&src)
            .is_some_and(|spans| !spans.iter().any(|(n, _, _)| n == "main"));
        if isolatable {
            continue;
        }
        for f in corpus_fns(&src) {
            if f.asserts > 0 && !f.expect_error {
                bad.push(format!(
                    "{}:{} fn {} — {} assertion(s) that can never run",
                    entry.display(),
                    f.line,
                    f.name,
                    f.asserts
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "these functions assert in a file that stops at its expected error, so nothing \
         they claim is ever checked. This file has a `main`, so the harness cannot isolate \
         the refusing cell (it runs `main` only, and a helper `main` calls cannot be \
         removed) — either drop the `main` so each cell is its own entry point and the \
         cells run independently, or move these to a companion file that RUNS (the \
         `<n>b-…` convention):\n  {}",
        bad.join("\n  ")
    );
}

/// R2 — an assertion in a function nothing calls is a claim nobody checks.
///
/// When a file has a `main`, both runners execute ONLY `main` (`run_test` here,
/// `prepare_native_test` there): every other zero-parameter function is compiled and
/// dropped.  `05-enums.loft` and `06-structs.loft` carried twenty-one assertions that way
/// — struct formatting, `limit()` fields, `&`-default parameters, copy-on-bind — none of
/// which had run in the life of those files.
///
/// Only zero-parameter functions are asked: one WITH parameters is a helper, reached
/// through whatever calls it.
#[test]
fn every_assertion_is_reachable_from_the_entry_point() {
    let mut bad: Vec<String> = Vec::new();
    for entry in corpus_files() {
        let Ok(src) = std::fs::read_to_string(&entry) else {
            continue;
        };
        let fns = corpus_fns(&src);
        if !fns.iter().any(|f| f.name == "main") {
            continue; // no `main`: every zero-parameter function is an entry point
        }
        for f in &fns {
            if f.name == "main" || !f.zero_param || f.asserts == 0 || f.expect_error {
                continue;
            }
            let called = src
                .lines()
                .enumerate()
                .filter(|(i, _)| *i + 1 != f.line)
                .any(|(_, l)| {
                    let l = l.trim_start();
                    !l.starts_with("//") && l.contains(&format!("{}(", f.name))
                });
            if !called {
                bad.push(format!(
                    "{}:{} fn {} — {} assertion(s), and `main` never calls it",
                    entry.display(),
                    f.line,
                    f.name,
                    f.asserts
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "a file with a `main` runs ONLY `main`, so these assertions are compiled and \
         never executed — call them from `main` or delete them:\n  {}",
        bad.join("\n  ")
    );
}

/// Scripts that have a dedicated `#[test] #[ignore]` wrapper.
/// Removed once the feature lands and the #[ignore] is dropped.
fn ignored_scripts() -> HashSet<&'static str> {
    HashSet::new()
}

/// Part B leak gate — script/doc files with KNOWN, pre-existing store leaks at
/// program exit (matched by file name; covers both `tests/scripts/` via
/// `loft_suite` and `tests/docs/` via the `dir`/`last` tests, which all run
/// through `run_test`).  Each leaks only top-level `main` locals (structs /
/// `main_vector<…>` / a `File`/`Parser` handle) that aren't scope-freed at the
/// very end of the program — benign for a one-shot run (the process exits
/// immediately after), but a real scope-free gap worth a later audit (@P322).
/// Grandfathered here so `run_test`'s leak gate catches NEW leaks (regressions)
/// without churning these.  A file is removed once its program-end frees are
/// tightened.
///
/// 2026-05-23 audit (@P322): nine of the original ten entries were
/// false-positives — scripts that abort mid-main via `raise(OOB / DivByZero /
/// AssertFailed)` so the dispatch loop short-circuits before scope-cleanup ops
/// emit (06-structs / 11-vectors / 37-stress / 93-vector-advanced /
/// 96-slot-assign / 07-vector / 16-parser / 15-lexer / 23-safety).  The new
/// `runtime_error.is_none()` gate (below) skips the leak check for those, so
/// they no longer need grandfathering.  The remaining real leak —
/// `51-coroutines.loft` leaking the `[10, 20, 30]` literal from
/// `nums_p219()`'s for-yield body — was FIXED 2026-05-23 by
/// `scopes::insert_free` emitting outer-scope frees before a nested
/// Void-block's terminal `Return`.  List is now empty; the leak gate
/// fails on any NEW leak.
const SCRIPTS_LEAK_ALLOW: &[&str] = &[];

/// Per-package library tests that BOTH the interpreter (`library_suite`) and
/// native (`tests/native.rs::native_library_suite`) suites skip, keyed by
/// `"<pkg>/<file>.loft"`, each with a one-line rationale.  Mirrors
/// `tests/native.rs::SCRIPTS_NATIVE_SKIP`.  Reserved for tests that need a GL
/// display or block forever — none today (no lib test creates a window or runs
/// an unbounded poll/accept loop; the `server` test is listen+close).
const LIB_TESTS_SKIP: &[&str] = &[
    // (empty) The network tests of the `web` fixture live in `tests-network/`, a
    // directory no suite walks (TESTING.md § Every skip says why, how it runs
    // instead, and when it ends), so no entry is needed for them.  An entry here
    // needs the open issue that explains it and the condition that removes it.
];

/// Library packages skipped wholesale (chunk-level), with rationale.  The
/// in-process suite aborts on the first SIGSEGV, so a chunk with multiple
/// interpreter crashes can't be run file-by-file here.
const LIB_PKGS_SKIP: &[&str] = &[
    // (empty) An entry here names a package whose tests crash the in-process
    // suite, the open issue that explains it, and the condition that removes it.
];

/// Returns true if `entry` (a `lib/<pkg>/tests/<file>.loft` path) is in the
/// shared skip-list.  Public so `tests/native.rs` reuses the same keying.
pub fn lib_test_skipped(entry: &std::path::Path) -> bool {
    let file = entry
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let pkg = entry
        .parent()
        .and_then(|d| d.parent())
        .and_then(|d| d.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if LIB_PKGS_SKIP.contains(&pkg.as_str()) {
        return true;
    }
    let key = format!("{pkg}/{file}");
    LIB_TESTS_SKIP.contains(&key.as_str())
}

/// Collect every `lib/<pkg>/tests/*.loft` path, sorted.  Shared discovery for
/// the interp + native library suites so they cover an identical set.
pub fn collect_library_tests() -> std::io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = Vec::new();
    for pkg in std::fs::read_dir("lib")?.filter_map(|e| e.ok()) {
        // Skip dot-dirs — `run_lib_test_in_temp_cwd` creates `.loft_test_tmp_*`
        // sibling dirs inside lib/ for artifact isolation; they must never be
        // discovered as packages.
        if pkg.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let tests_dir = pkg.path().join("tests");
        if !tests_dir.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(&tests_dir)?.filter_map(|e| e.ok()) {
            let p = f.path();
            if p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            {
                files.push(p);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Run `loft [extra_args] test <stem>` for a lib package in a UNIQUE temp CWD so
/// the interpreter `library_suite` and the native `native_library_suite` (separate
/// test binaries, run concurrently by nextest) don't race on cwd-relative test
/// artifacts (e.g. `moros_render_test.glb`, which several tests write+`delete` in
/// the package dir).  The temp dir is a `.loft_test_tmp_*` SIBLING inside `lib/`
/// so the package's relative deps (`../<name>`) still resolve to the real
/// packages; the package's contents are symlinked in, and cwd-relative artifacts
/// land in the unique dir, which is removed afterwards.  Non-unix falls back to
/// the package dir (Windows is gated; symlinks need privileges there).
pub fn run_lib_test_in_temp_cwd(
    loft_bin: &str,
    pkg_dir: &Path,
    stem: &str,
    extra_args: &[&str],
) -> std::io::Result<std::process::Output> {
    let mut args: Vec<&str> = extra_args.to_vec();
    args.push("test");
    args.push(stem);
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
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir(&tmp)?;
        for entry in std::fs::read_dir(pkg_dir)?.filter_map(|e| e.ok()) {
            let target = entry.path().canonicalize().unwrap_or_else(|_| entry.path());
            let _ = std::os::unix::fs::symlink(&target, tmp.join(entry.file_name()));
        }
        let out = std::process::Command::new(loft_bin)
            .current_dir(&tmp)
            .args(&args)
            .output();
        let _ = std::fs::remove_dir_all(&tmp);
        out
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new(loft_bin)
            .current_dir(pkg_dir)
            .args(&args)
            .output()
    }
}

/// Gate every `lib/<pkg>/tests/*.loft` through the INTERPRETER.  Before this the
/// libraries had no CI coverage of their own (`lib/*/tests/` was referenced by
/// nothing).
///
/// Each test runs as a SUBPROCESS via the package-aware `loft test` subcommand
/// (`cd lib/<pkg> && loft test <file>`), exactly as `make test-packages` does.
/// Subprocessing is deliberate: it isolates a crashing lib test (a SIGSEGV
/// reports as that file's failure instead of aborting the whole suite) and
/// reuses `loft test`'s correct package resolution + native-extension loading
/// (an in-process harness mis-resolves intra-package `use` and skips
/// `extensions::load_all`).  The native counterpart is
/// `tests/native.rs::native_library_suite`.
// @speed 7.9
#[test]
fn library_suite() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    let mut failures: Vec<String> = Vec::new();
    let mut ran = 0;
    for entry in collect_library_tests()? {
        if lib_test_skipped(&entry) {
            println!("skip {entry:?} (LIB_TESTS_SKIP / LIB_PKGS_SKIP)");
            continue;
        }
        let pkg_dir = entry.parent().and_then(|d| d.parent()).unwrap_or(&entry);
        let stem = entry
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        println!("lib test {entry:?}");
        let out = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, &stem, &[])?;
        ran += 1;
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        // `loft test` exits 0 even on a CAUGHT SIGSEGV (the crash handler prints
        // and returns), so detect crashes by their printed marker too.
        let failed = !out.status.success()
            || combined.contains("SIGSEGV")
            || combined.contains("panicked")
            || combined.contains("test result: FAILED")
            || !combined.contains("test result: ok");
        if failed {
            let tail: Vec<&str> = combined.lines().rev().take(4).collect();
            failures.push(format!("{entry:?}: {}", tail.join(" | ")));
        }
    }
    if !failures.is_empty() {
        return Err(Error::other(format!(
            "{} of {ran} library tests failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        )));
    }
    println!("library_suite: {ran} library tests passed");
    Ok(())
}

macro_rules! script_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() -> std::io::Result<()> {
            let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            run_test(PathBuf::from($path), false, false)
        }
    };
}

script_test!(integers, "tests/scripts/01-integers.loft");
script_test!(floats, "tests/scripts/02-floats.loft");
script_test!(text, "tests/scripts/03-text.loft");
script_test!(booleans, "tests/scripts/04-booleans.loft");
script_test!(enums, "tests/scripts/05-enums.loft");
script_test!(structs, "tests/scripts/06-structs.loft");
script_test!(control_flow, "tests/scripts/07-control-flow.loft");
script_test!(functions, "tests/scripts/08-functions.loft");
script_test!(lambdas, "tests/scripts/09-lambdas.loft");
script_test!(vectors, "tests/scripts/11-vectors.loft");
script_test!(collections, "tests/scripts/12-collections.loft");
script_test!(map_filter_reduce, "tests/scripts/13-map-filter-reduce.loft");
script_test!(formatting, "tests/scripts/14-formatting.loft");
script_test!(min_max_clamp, "tests/scripts/17-min-max-clamp.loft");
script_test!(math_functions, "tests/scripts/18-math-functions.loft");
script_test!(files, "tests/scripts/19-files.loft");
#[test]
fn binary() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/scripts/20-binary.loft"), false, false)
}
script_test!(binary_ops, "tests/scripts/21-binary-ops.loft");
script_test!(script_threading, "tests/scripts/22-threading.loft");
script_test!(stress, "tests/scripts/37-stress.loft");
script_test!(single_type, "tests/scripts/52-single.loft");

// S16a: field-name overlap between two plain structs in the same file.
// Both structs share a field name `val` at different byte offsets.
// Exercises sorted lookup, range query, full iteration, and index range query.
// Confirmed working — field offsets are type-scoped in determine_keys().
script_test!(
    field_overlap_structs,
    "tests/scripts/23-field-overlap-structs.loft"
);

// S16a: field-name overlap involving struct-enum variants in the same file.
// Scenario A: two struct-enum variants share field `score` at different offsets.
// Scenario B: a plain struct and a struct-enum variant share field `key`.
script_test!(
    field_overlap_enum_struct,
    "tests/scripts/24-field-overlap-enum-struct.loft"
);

// S16b: range queries on sorted<EnumVariant[field]>
// Fixed: index_type() now returns Type::Reference(variant_def_nr) instead of
// Type::Enum(parent, true) so for_type() and field access work correctly.
script_test!(
    sorted_enum_variant_range,
    "tests/scripts/25-sorted-enum-variant-range.loft"
);

// Logging functions compile and run as no-ops without a log.conf.
script_test!(logging_script, "tests/scripts/53-logging.loft");

// Implicit type widening in mixed integer/long/float expressions.
script_test!(auto_convert, "tests/scripts/54-auto-convert.loft");

// stack_trace() introspection returns frames from nested calls.
script_test!(stack_trace_script, "tests/scripts/55-stack-trace.loft");
script_test!(
    p430_file_builtins,
    "tests/scripts/430-native-file-builtins.loft"
);

// @PLAN53 Wave 2 regression: remove_vector off-by-one OOB.
// A vector at full initial capacity (11 integer elements) with a middle element
// removed triggered a copy_block read one past the last valid element.
// Fixed in commit 72113508; this exercises the boundary case so the ASan CI
// job catches any reintroduction.
script_test!(
    plan53_remove_vector_oob,
    "tests/scripts/162-plan53-remove-vector-oob.loft"
);

/// P89 regression: every field name `n_stack_trace` looks up at runtime
/// must exist in the loaded schema.  If `default/04_stacktrace.loft` is
/// edited to rename or remove a field, this test fails immediately
/// instead of silently producing garbage stack-trace records.
#[test]
fn p89_stacktrace_schema_fields_exist() {
    let (_data, db) = cached_default();
    let db = &db;
    let sf_fields = [
        ("StackFrame", "function"),
        ("StackFrame", "file"),
        ("StackFrame", "line"),
        ("StackFrame", "arguments"),
        ("StackFrame", "variables"),
        ("VarInfo", "name"),
        ("VarInfo", "type_name"),
        ("VarInfo", "value"),
        ("BoolVal", "b"),
        ("IntVal", "n"),
        ("LongVal", "n"),
        ("FloatVal", "f"),
        ("SingleVal", "f"),
        ("CharVal", "c"),
        ("TextVal", "t"),
        ("RefVal", "store"),
        ("RefVal", "rec"),
        ("RefVal", "pos"),
        ("OtherVal", "description"),
    ];
    for (ty, field) in sf_fields {
        let tp = db.name(ty);
        assert_ne!(
            tp,
            u16::MAX,
            "schema is missing type {ty} (default/04_stacktrace.loft drift)"
        );
        let pos = db.position(tp, field);
        assert_ne!(
            pos,
            u16::MAX,
            "schema is missing {ty}.{field} (default/04_stacktrace.loft drift)"
        );
    }
}

/// Quick iteration test: run only the final suite file (`16-parser.loft`) without
/// regenerating documentation.  Use this during active development on the parser
/// to get a fast feedback cycle.
#[test]
fn last() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/docs/16-parser.loft"), false, true)
}

/// Regression test for P136 — SIGSEGV / heap-corruption on
/// `tests/scripts/79-null-early-exit.loft`.  Root cause was in
/// `state/codegen.rs::gen_if`: when the true branch diverges (C56
/// `?? return` desugars to `if (is_null) { return ret } else null`)
/// and `f_val == Null`, `stack.position` was left at the true branch's
/// end-state instead of being reset to the pre-if value.  Runtime then
/// reached the join point (via goto-false) with a smaller stack_pos
/// than codegen expected, and every subsequent Var/Put read/wrote four
/// bytes off — eventually clobbering the return-address slot and
/// looping into an 8008-byte stack overflow.
#[test]
fn sigsegv_repro_79_alone() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/79-null-early-exit.loft"),
        false,
        false,
    )
}

/// Run `17-libraries.loft` in isolation; verifies inline-ref chaining (T0-6 fix).
#[test]
fn libraries() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/docs/17-libraries.loft"), false, true)
}

/// Run `19-threading.loft` in isolation; covers `parallel_for_int` and the
/// new compiler-checked `parallel_for` API with `fn` references.
#[test]
fn threading() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/docs/19-threading.loft"), false, true)
}

/// Run `20-logging.loft` in isolation; verifies log_* functions compile and
/// can be called without aborting when no logger is configured.
#[test]
fn logging() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/docs/20-logging.loft"), false, true)
}

/// Debug the run of `13-file.loft` with a full execution trace written to
/// `tests/dumps/13-file.loft.txt`.  Use this to diagnose store-allocation bugs.
#[test]
fn file_debug() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/docs/13-file.loft"), true, true)
}

/// Debug the run of `16-parser.loft` with a full execution trace written to
/// `tests/dumps/16-parser.loft.txt`.  Use this when diagnosing parser regressions.
#[test]
fn parser_debug() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/docs/16-parser.loft"), true, true)
}

/// Verify that `fn main(args: vector<text>)` receives the arguments passed via `execute_argv`.
#[test]
fn main_argv() {
    let code = r#"
fn main(args: vector<text>) {
    assert(len(args) == 3, "expected 3 args");
    assert(args[0] == "hello", "arg 0");
    assert(args[1] == "world", "arg 1");
    assert(args[2] == "foo", "arg 2");
}
"#;
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(code, "main_argv", false);
    for l in p.diagnostics.lines() {
        println!("{l}");
    }
    assert!(p.diagnostics.is_empty(), "parse errors");
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let args = ["hello", "world", "foo"]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    state.execute_argv("main", &p.data, &args);
}

/// T2: Verify `size()` returns Unicode code-point count, not byte length.
#[test]
fn size_text() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/41-size-text.loft"),
        false,
        true,
    )
}

/// A10: Verify field iteration (for f in s#fields).
#[test]
fn field_iteration() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/45-field-iter.loft"),
        false,
        true,
    )
}

/// L2: Verify nested match patterns in field positions.
#[test]
fn nested_match() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/44-nested-match.loft"),
        false,
        true,
    )
}

/// P3: Verify vector aggregates (sum, min_of, max_of, any, all, count_if).
#[test]
fn aggregates() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/43-aggregates.loft"),
        false,
        true,
    )
}

/// L3: Verify FileResult enum for filesystem operations.
#[test]
fn file_result() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/42-file-result.loft"),
        false,
        true,
    )
}

/// P5.2: Verify generic function call-site instantiation.
#[test]
fn generics() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/scripts/48-generics.loft"), false, true)
}

/// L7: Verify init(expr) stored field initialiser with $ reference.
#[test]
fn init_fields() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/49-init-fields.loft"),
        false,
        true,
    )
}

/// Regression tests for documented caveats (CAVEATS.md).
#[test]
fn caveats() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/scripts/46-caveats.loft"), false, true)
}

/// Parse, type-check, compile, and execute one `.loft` test file.
///
/// The default library in `default/` is loaded first, then `entry` is parsed on
/// Extract `// #warn <text>` declarations from a `.loft` source file.
///
/// Each matching comment declares that the script is expected to produce a
/// `Warning:` diagnostic whose message contains `<text>` as a substring.
/// Lines of the form `// #warn Parameter 'x' does not need to be a reference`
/// allow a script that intentionally triggers a warning to still pass.
fn expected_warnings(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            t.strip_prefix("// #warn ").map(|s| s.trim().to_string())
        })
        .collect()
}

/// Validate diagnostics against expected patterns from `// #warn`, `@EXPECT_ERROR`,
/// and `@EXPECT_WARNING` declarations.
///
/// Returns `Ok(())` when every diagnostic matches a declared pattern.
/// Returns `Err` when any diagnostic is unexpected.  All mismatches are printed.
fn check_diagnostics(
    diagnostics: &[String],
    expected_warns: &[String],
    expected_errors: &[String],
    expected_ann_warnings: &[String],
    final_pass: bool,
) -> std::io::Result<()> {
    let mut unmatched_warns: Vec<&str> = expected_warns.iter().map(String::as_str).collect();
    let mut unmatched_errors: Vec<&str> = expected_errors.iter().map(String::as_str).collect();
    let mut unmatched_ann_warns: Vec<&str> =
        expected_ann_warnings.iter().map(String::as_str).collect();
    let mut unexpected: Vec<&str> = Vec::new();

    for diag in diagnostics {
        if matches!(
            loft::diagnostics::compact_level(diag),
            Some(loft::diagnostics::Level::Debug)
        ) {
            continue;
        } else if matches!(
            loft::diagnostics::compact_level(diag),
            Some(loft::diagnostics::Level::Warning | loft::diagnostics::Level::Advice)
        ) {
            // Advice counts for `// #warn` / `@EXPECT_WARNING` the same as a warning:
            // the declaration asserts a diagnostic FIRED, not which tier it landed in.
            // Without this an `Advice:` line falls to the error branch below and a
            // script that deliberately triggers a deprecation fails as "unexpected".
            // Try #warn patterns first (strict — must all match)
            if let Some(pos) = unmatched_warns.iter().position(|pat| diag.contains(*pat)) {
                println!("expected warning matched: {diag}");
                unmatched_warns.remove(pos);
            } else if let Some(pos) = unmatched_ann_warns
                .iter()
                .position(|pat| diag.contains(*pat))
            {
                println!("expected @EXPECT_WARNING matched: {diag}");
                unmatched_ann_warns.remove(pos);
            }
            // Other warnings are not fatal — just log them.
        } else if let Some(pos) = unmatched_errors.iter().position(|pat| diag.contains(*pat)) {
            println!("expected @EXPECT_ERROR matched: {diag}");
            unmatched_errors.remove(pos);
        } else {
            println!("unexpected error: {diag}");
            unexpected.push(diag);
        }
    }
    for pat in &unmatched_warns {
        println!("expected warning not emitted: {pat}");
    }
    // An `@EXPECT_ERROR` / `@EXPECT_WARNING` that never matched is a guard that no longer
    // guards anything — the diagnostic it pins may have been reworded, narrowed, or
    // removed, and the file still passed.  Both used to be collected and then DROPPED,
    // and 56 of the 167 `@EXPECT_ERROR` annotations in the tree were inert when that was
    // measured (loft#929).  Failing on them is what stops the guard rotting again.
    //
    // The commonest way one goes inert is not a reworded message: `Parser::parse` runs
    // pass 2 only when pass 1 finished clean, so ONE pass-1 error silences every
    // `!first_pass` diagnostic in the same file.  A file therefore asserts pass-1 errors
    // OR pass-2 errors, never both — see the 102/102b and 36/36b pairs.
    //
    // Reported on the FINAL check only: before it, a diagnostic still to come reads as
    // one that will never arrive.
    if final_pass {
        for pat in &unmatched_errors {
            println!("expected error not emitted: {pat}");
        }
        for pat in &unmatched_ann_warns {
            println!("expected @EXPECT_WARNING not emitted: {pat}");
        }
    }
    // An unmatched expectation only condemns the file on the FINAL check.  The parse-time
    // call runs before the post-scope-check lints have spoken, and `894-lost-write-through
    // -returned-struct.loft`'s `@EXPECT_WARNING` names one of those — failing there would
    // report a diagnostic as missing while it was still to come.
    if unexpected.is_empty()
        && unmatched_warns.is_empty()
        && (!final_pass || (unmatched_errors.is_empty() && unmatched_ann_warns.is_empty()))
    {
        Ok(())
    } else {
        Err(Error::from(std::io::ErrorKind::InvalidData))
    }
}

/// Collect the names of all zero-parameter user functions defined in `data`
/// starting from definition `start_def`.  Returns `"n_<name>"` internal names
/// stripped to their user-facing form (e.g. `"main"`, `"test_foo"`).
///
/// This mirrors the discovery logic in `test_runner.rs` so that scripts using
/// `fn test_*()` style entry points are exercised by `cargo test`, not only by
/// `loft --tests`.
fn entry_point_names(data: &Data, start_def: u32) -> Vec<String> {
    let mut names = Vec::new();
    for d_nr in start_def..data.definitions() {
        let def = data.def(d_nr);
        // `Definition::is_corpus_entry_point` is the ONE answer, shared with
        // `tests/native.rs` — the two halves of a differential that run different sets of
        // functions cannot report a divergence between them (loft#1293).
        if !def.is_corpus_entry_point() {
            continue;
        }
        let user_name = def.name.strip_prefix("n_").unwrap_or(&def.name);
        names.push(user_name.to_string());
    }
    names
}

/// Compile and run a single `.loft` script test.
///
/// Scripts may declare expected compile-time warnings with `// #warn <text>`
/// comments.  Each such comment consumes one matching `Warning:` diagnostic.
/// Unexpected diagnostics (errors or unmatched warnings) fail the test.
///
/// Any parse or type errors are printed and immediately fail the test.
/// On success the bytecode is generated and every zero-parameter user function
/// is called (not just `main`).  This ensures scripts that use `fn test_*()`
/// entry points are also exercised by `cargo test` / `cargo llvm-cov`.
///
/// Each entry-point function is run inside `catch_unwind` so that a failing
/// assert in one function does not abort the remaining functions.  All failures
/// are collected and reported at the end.
///
/// When `debug` is true (debug builds only) a human-readable bytecode dump is
/// written to `tests/dumps/<filename>.txt` and the interpreter emits a full
/// execution trace to that file.  Set `LOFT_DUMP=1` to get the bytecode dump
/// without the execution trace for any non-debug test invocation.
/// Scan source for `// @EXPECT_FAIL` annotations bound to specific functions.
/// Returns a set of function names whose panics should be tolerated.
/// Also returns true if the file has a file-level `@EXPECT_FAIL`.
///
/// One parser, shared with the native runner — see [`common::expect_fail_fns`].
fn expect_fail_fns(source: &str) -> (HashSet<String>, bool) {
    common::expect_fail_fns(source)
}

/// Collect all `// @EXPECT_ERROR:` and `// @EXPECT_WARNING:` annotation substrings.
/// These are treated as expected diagnostics and consumed by `check_diagnostics`.
fn expected_annotations(source: &str) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = common::expect_tag(trimmed, "@EXPECT_ERROR:") {
            let sub = rest.trim();
            if !sub.is_empty() {
                errors.push(sub.to_string());
            }
        } else if let Some(rest) = common::expect_tag(trimmed, "@EXPECT_WARNING:") {
            let sub = rest.trim();
            if !sub.is_empty() {
                warnings.push(sub.to_string());
            }
        }
    }
    (errors, warnings)
}

#[cfg_attr(not(debug_assertions), allow(unused_variables, unused_mut))]

/// The 1-based source lines an ERROR-level diagnostic points at, for `path`.
///
/// The compact renderer the harness pins ends every diagnostic with ` at <file>:<line>:<col>`,
/// which is what makes attribution possible at all. A diagnostic naming another file (the
/// stdlib, an `@ARGS --lib` fixture) is skipped: it cannot be isolated by editing THIS source.
fn error_lines_in(diags: &[String], path: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for d in diags {
        if !matches!(
            loft::diagnostics::compact_level(d),
            Some(loft::diagnostics::Level::Error)
        ) {
            continue;
        }
        // `… at /abs/path.loft:12:3` — take the LAST such marker, since a message may
        // quote a path of its own.
        if let Some(at) = d.rfind(&format!("{path}:")) {
            let tail = &d[at + path.len() + 1..];
            let line: String = tail.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(n) = line.parse::<usize>() {
                out.push(n);
            }
        }
    }
    out
}

/// Top-level `fn NAME(…) { … }` spans as `(name, first_line, last_line)`, 1-based inclusive.
///
/// Returns `None` the moment the scan is not certain — an unbalanced file, a `fn` whose body
/// never closes. A wrong span would silently remove the wrong cell, which is worse than not
/// isolating at all, so every ambiguity falls back to the old behaviour.
fn top_level_fn_spans(src: &str) -> Option<Vec<(String, usize, usize)>> {
    top_level_fn_spans_full(src).map(|v| v.into_iter().map(|(n, a, b, _)| (n, a, b)).collect())
}

/// [`top_level_fn_spans`] plus, per function, whether isolating the file would actually RUN an
/// assertion of that function: it takes no parameters (so the harness runs it as an entry point)
/// and its body contains an `assert(`.  A file with no such survivor is not worth a second
/// parse, and one whose survivors all take parameters makes `entry_point_names` assert rather
/// than return empty — so the isolation has to know before it commits (loft#1242).
fn top_level_fn_spans_full(src: &str) -> Option<Vec<(String, usize, usize, bool)>> {
    let lines: Vec<&str> = src.lines().collect();
    let mut spans = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let t = lines[i].trim_start();
        let is_fn = t.starts_with("fn ") || t.starts_with("pub fn ");
        if !is_fn {
            i += 1;
            continue;
        }
        let name: String = t
            .trim_start_matches("pub ")
            .trim_start_matches("fn ")
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        // Brace-match from here, ignoring braces inside line comments and string literals.
        // loft interpolates with `{…}` INSIDE strings, so skipping string bodies is not an
        // optimisation — a counter that saw them would close the function early.
        let mut depth = 0i32;
        let mut opened = false;
        let mut j = i;
        while j < lines.len() {
            let mut in_str = false;
            let mut prev_backslash = false;
            let bytes: Vec<char> = lines[j].chars().collect();
            let mut k = 0usize;
            while k < bytes.len() {
                let c = bytes[k];
                if in_str {
                    if c == '\\' && !prev_backslash {
                        prev_backslash = true;
                        k += 1;
                        continue;
                    }
                    if c == '"' && !prev_backslash {
                        in_str = false;
                    }
                    prev_backslash = false;
                } else if c == '"' {
                    in_str = true;
                } else if c == '/' && k + 1 < bytes.len() && bytes[k + 1] == '/' {
                    break; // rest of the line is a comment
                } else if c == '{' {
                    depth += 1;
                    opened = true;
                } else if c == '}' {
                    depth -= 1;
                    if depth < 0 {
                        return None;
                    }
                }
                k += 1;
            }
            if opened && depth == 0 {
                // `fn name()` — empty parens on the declaring line is the whole test; a
                // parameter list that wraps lines is not a shape this corpus uses, and a
                // false "has parameters" only costs the fallback.
                let zero_param = lines[i].contains(&format!("{name}()"));
                // "Would isolating this actually run an assertion?" is the real question, and
                // it is not the same as "is this an entry point": `1215`'s only surviving
                // function after blanking is the helper `mk1215() -> float { 2.5 }`, which is a
                // zero-param entry point that asserts nothing.  Isolating there costs a second
                // parse to run a cell that cannot fail.
                let asserts = lines[i..=j].iter().any(|l| l.contains("assert("));
                spans.push((name, i + 1, j + 1, zero_param && asserts));
                break;
            }
            j += 1;
        }
        if !(opened && depth == 0) {
            return None; // never closed — do not guess
        }
        i = j + 1;
    }
    Some(spans)
}

/// `src` with every function an ERROR points into blanked out, or `None` when the file cannot
/// be isolated that way.
///
/// The point (loft#1242): a file should be able to assert a REFUSAL and still run the cells the
/// refusal has nothing to do with. Until now one `@EXPECT_ERROR` stopped the whole file before
/// execution, so a companion `<n>b-…` file had to exist purely to hold the runnable half — and
/// the split is not free, because the two halves drift and a reader has to find both.
///
/// Blanking rather than deleting keeps every LINE NUMBER stable, so a diagnostic from the second
/// parse still points where the reader expects.
///
/// Returns `None` — keeping the old "errors consumed" behaviour — when:
///   * the file has a `main`, because the harness then runs only `main` and a helper it calls
///     cannot be removed without changing what `main` means;
///   * an error sits outside every function (a file-scope refusal), which cannot be isolated;
///   * removing the failing cells would leave nothing to run;
///   * the brace scan was not certain.
fn source_without_failing_fns(src: &str, diags: &[String], path: &str) -> Option<String> {
    let full = top_level_fn_spans_full(src)?;
    let spans: Vec<(String, usize, usize)> = full
        .iter()
        .map(|(n, a, b, _)| (n.clone(), *a, *b))
        .collect();
    if spans.iter().any(|(n, _, _)| n == "main") {
        return None;
    }
    let lines = error_lines_in(diags, path);
    if lines.is_empty() {
        return None;
    }
    let mut failing: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    for l in &lines {
        // A refusal outside any function cannot be isolated — `?` returns `None` and the
        // caller falls back to the old behaviour.
        let idx = spans.iter().position(|(_, a, b)| *l >= *a && *l <= *b)?;
        failing.insert(idx);
    }
    if failing.len() == spans.len() {
        return None; // every cell failed; there is nothing left to run
    }
    // …and something that SURVIVES must be worth running: a zero-parameter cell that carries an
    // assertion.  Two measured shapes make this the right test rather than "is there an entry
    // point".  `1103b`'s surviving function takes parameters, so the runner finds NO entry point
    // and asserts rather than returning; `1215`'s survivor is `mk1215() -> float { 2.5 }`, an
    // entry point that claims nothing.  Neither is worth a second parse, and the first one
    // crashes the suite.
    if !full
        .iter()
        .enumerate()
        .any(|(i, (_, _, _, runnable))| *runnable && !failing.contains(&i))
    {
        return None;
    }
    // ⚠ A blanked cell must not be REACHABLE from a surviving one.
    //
    // Isolation is only sound while the cells are independent. If a surviving function calls a
    // blanked one, removing it does not narrow the file — it changes what the survivor MEANS,
    // and the most likely way that shows up is an assertion quietly becoming a no-op, which
    // reads green. That is the failure this whole change exists to remove, so re-introducing it
    // one level down would be the worst possible trade.
    //
    // Conservative by name: any mention of `<name>(` outside the blanked spans is treated as a
    // call and stops the isolation. It over-refuses (a comment naming the function counts), and
    // over-refusing costs only the fallback to the old behaviour, while under-refusing costs a
    // silent wrong pass.
    let survivor_text: String = src
        .lines()
        .enumerate()
        .filter(|(i, _)| {
            let line = i + 1;
            !failing
                .iter()
                .any(|idx| line >= spans[*idx].1 && line <= spans[*idx].2)
        })
        .map(|(_, l)| l)
        .collect::<Vec<_>>()
        .join("\n");
    for idx in &failing {
        let called = format!("{}(", spans[*idx].0);
        if survivor_text.contains(&called) {
            return None;
        }
    }
    let mut out: Vec<String> = src.lines().map(str::to_string).collect();
    for idx in &failing {
        let (_, a, b) = &spans[*idx];
        for l in *a..=*b {
            if l >= 1 && l <= out.len() {
                out[l - 1] = String::new();
            }
        }
    }
    Some(out.join("\n"))
}

fn run_test(entry: PathBuf, debug: bool, allow_dump: bool) -> std::io::Result<()> {
    let mut collected: Vec<String> = Vec::new();
    run_test_inner(entry, debug, allow_dump, None, 0, &mut collected)
}

/// How many times a file may be peeled before the harness gives up (loft#1242).
///
/// Each round blanks the cells that errored and re-parses, so a file needs one round per LAYER
/// of masking, not per error. Bounded because a pathological file could otherwise loop: the
/// limit is far above anything the corpus needs (the deepest real case is two).
const MAX_PEEL: u32 = 8;

/// @PLN154 phase 5 — did the stack shadow report anything while THIS script ran?
///
/// The count is process-wide (a `par` arm has its own `State` and its own frame), so the
/// per-script answer is a difference against the count before the script started.  Always
/// `false` when the shadow is not armed, which is every ordinary run.
fn stack_verify_gate(before: u64) -> bool {
    loft::stack_verify::enabled() && loft::stack_verify::violations() > before
}

/// `override_src` is the peeled re-run (loft#1242): the same file with the cells an error touched
/// blanked out, so the rest parses and runs. `depth` counts the peels; `collected` accumulates
/// EVERY round's diagnostics, because that union is what the declarations are checked against.
///
/// The union is the point, not an implementation detail. `Parser::parse` runs pass 2 only when
/// pass 1 finished clean, so ONE pass-1 error silences every `!first_pass` diagnostic in the file
/// — `Unknown variable`, the const/ref checks, match exhaustiveness, the whole N-Store family.
/// A file could therefore assert pass-1 errors OR pass-2 errors and never both, which is the
/// other half of why the `<n>b-…` split exists. Peeling the pass-1 cell lets the next round reach
/// pass 2, and checking the union means both are verified against one file's declarations.
///
/// Only depth 0 checks, and it checks after the recursion has returned — an inner round's
/// diagnostics have to be in the union before the verdict is taken.
///
/// ⚠ The two `allow`s are about `debug-assertions = false`, which `Cargo.toml` sets for the
/// `loft` package in EVERY profile: the only uses of `allow_dump`, and the only mutations of
/// `p_data`, are inside `#[cfg(debug_assertions)]` blocks, so with those compiled out clippy sees
/// a parameter that merely recurses and a binding never mutated. Both are real in a
/// debug-assertions build — silencing is right here, deleting them would break that build.
#[allow(clippy::only_used_in_recursion, unused_mut)]
fn run_test_inner(
    entry: PathBuf,
    debug: bool,
    allow_dump: bool,
    override_src: Option<String>,
    depth: u32,
    collected: &mut Vec<String>,
) -> std::io::Result<()> {
    // Idempotent: installs SIGSEGV/SIGABRT handler once per test
    // process so crashes print the last-executed opcode + PC.
    loft::crash_report::install("wrap");
    // @PLN154 phase 5 — the shadow's tally before this script, so the gate below reads the
    // difference rather than the process total.
    let shadow_before = loft::stack_verify::violations();
    println!("run {entry:?}");
    // `loft_suite` runs the whole corpus in ONE process, so a fault that depends
    // on accumulated arena state (loft#920) has to be attributed to the script
    // that triggered it.  The line above goes to stdout while every runtime
    // complaint goes to stderr, and the two are buffered independently — reading
    // an interleaving of them names the wrong script.  `LOFT_TRACE_SCRIPT=1` puts
    // the marker on the SAME stream as the complaint, where the order is real.
    if std::env::var_os("LOFT_TRACE_SCRIPT").is_some() {
        eprintln!("run {entry:?}");
    }
    let source = match override_src {
        Some(ref t) => t.clone(),
        None => std::fs::read_to_string(&entry)?,
    };
    let expected = expected_warnings(&source);
    let (exp_errors, exp_ann_warns) = expected_annotations(&source);
    let (expect_fail, file_level_fail) = expect_fail_fns(&source);
    let _has_expected_errors = !exp_errors.is_empty();
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    // Honour `// @ARGS: --lib <dir>` lines at the top of the test file so
    // scripts using `use foo` / `use foo::*` can locate fixtures in
    // `tests/lib/`.  Same parser as `tests/native.rs:252-265` — the two
    // runners are now symmetric on the `@ARGS` convention.  Any other
    // flag in the @ARGS line (e.g. `--path`, `--html`) is ignored at
    // this layer.
    for line in source.lines().take(20) {
        if let Some(args) = line.trim().strip_prefix("// @ARGS:") {
            let mut tokens = args.split_whitespace();
            while let Some(tok) = tokens.next() {
                if tok == "--lib"
                    && let Some(dir) = tokens.next()
                {
                    p.lib_dirs.push(dir.to_string());
                }
            }
        }
    }
    #[cfg(debug_assertions)]
    let types = p.database.types.len();
    let start_def = p.data.definitions();
    let path = entry.to_string_lossy().to_string();
    // loft#1242 — the isolated re-run parses the REDUCED TEXT, not the file. `parse` reads from
    // disk, so overriding `source` alone left the second pass re-parsing the original and
    // re-reporting the very error it was meant to step around. `parse_str` is handed the same
    // `path` as its filename so diagnostics and `@ARGS` positions still name the real file, and
    // the blanking preserved line numbers so those positions stay true.
    if let Some(ref text) = override_src {
        p.parse_str(text, &path, false);
    } else {
        let _ = p.parse(&path, false);
    }
    let had_errors = !p.diagnostics.is_empty()
        && p.diagnostics.lines().iter().any(|l| {
            !matches!(
                loft::diagnostics::compact_level(l),
                Some(
                    loft::diagnostics::Level::Warning
                        | loft::diagnostics::Level::Debug
                        | loft::diagnostics::Level::Advice
                )
            )
        });
    // Run the check whenever the file DECLARES something, not only when the parser said
    // something.  Gating on `!p.diagnostics.is_empty()` was the second way an expectation
    // went inert: a file whose diagnostics disappeared entirely never reached
    // `check_diagnostics`, so its `@EXPECT_ERROR` lines were not merely unmatched — they
    // were never looked at (loft#929).
    // ⚠ Each round contributes to the union EXACTLY ONCE, and where matters. The post-compile
    // block below re-reads the same diagnostics (`std::mem::take` of `p.diagnostics`, plus the
    // lints), so extending here as well would double every parse diagnostic — and a duplicate
    // has no second `@EXPECT_ERROR` to match, so `check_diagnostics` would report it as
    // unexpected and fail the file. The extends therefore sit on the three paths that LEAVE this
    // function: before recursing, before the errors-consumed return, and in the post-compile
    // block, which already includes the parse diagnostics it took.
    // Decide whether to peel BEFORE any verdict: an inner round's diagnostics must be in the
    // union first, or a pass-2 error unmasked by peeling would be reported as unexpected.
    let reduced = if had_errors && depth < MAX_PEEL {
        source_without_failing_fns(&source, &p.diagnostics.lines(), &path)
    } else {
        None
    };
    if let Some(text) = reduced {
        println!("  errors consumed; running the cells they did not touch");
        collected.extend(p.diagnostics.lines());
        let ran = run_test_inner(
            entry.clone(),
            debug,
            allow_dump,
            Some(text),
            depth + 1,
            collected,
        );
        if depth == 0 {
            check_diagnostics(collected, &expected, &exp_errors, &exp_ann_warns, true)?;
        }
        return ran;
    }
    // Only skip execution when the file actually has unresolved parse errors.
    // If @EXPECT_ERROR annotations exist but the errors are gone (bug fixed),
    // proceed to execution so the fix can be verified.
    if had_errors {
        collected.extend(p.diagnostics.lines());
        if depth == 0 {
            check_diagnostics(collected, &expected, &exp_errors, &exp_ann_warns, true)?;
        }
        println!("  ok (errors consumed)");
        return Ok(());
    }
    // The whole-program lints, in the same window `src/main.rs` runs them: on the parsed
    // IR, after `Parser::parse` and BEFORE `scopes::check`.  Until now they ran only in
    // the CLI, so this suite could neither confirm one of their warnings nor catch a false
    // positive from one — `894-lost-write-through-returned-struct.loft` carried an
    // `@EXPECT_WARNING` for a diagnostic the harness had no way to produce (loft#929).
    // Same precondition as the CLI: an aborting error means resolution did not finish, so
    // every ownership verdict derived from it is known-bad.
    let mut diagnostics = std::mem::take(&mut p.diagnostics);
    if diagnostics.level() < loft::diagnostics::Level::Error {
        loft::use_analysis::warn_dead_stores(&p.data, &mut diagnostics, &path);
        loft::use_analysis::warn_double_move(&p.data, &mut diagnostics, &path);
        loft::use_analysis::warn_lost_temp_writes(&p.data, &mut diagnostics, &path);
        // loft#1397 — here for the reason the note above gives, and for the screen it buys:
        // a false positive from this lint would otherwise be invisible to the 1200-file
        // corpus, which is the only thing that reads that many programs.
        loft::use_analysis::warn_variant_overwritten(&p.data, &mut diagnostics, &path);
    }
    // Scope check and bytecode generation can panic on compiler bugs.
    // When the file has @EXPECT_FAIL annotations, tolerate the panic.
    let compile_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scopes::check(&mut p.data, &mut p.database);
        let mut state = State::new(p.database);
        byte_code(&mut state, &mut p.data);
        (state, p.data)
    }));
    let (mut state, mut p_data) = match compile_result {
        Ok(pair) => {
            if file_level_fail {
                println!("  FIXED {path} (was @EXPECT_FAIL, now compiles)");
            }
            pair
        }
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else {
                "unknown panic".to_string()
            };
            if file_level_fail || !expect_fail.is_empty() {
                println!("  expected compile fail {path} — {msg}");
                return Ok(());
            }
            std::panic::resume_unwind(payload);
        }
    };

    // @PLN163 P3 — the lease errors (`copy-of-droppable`, `read-after-move`) are raised after
    // the scope pass, where the CLI and `loft test` raise them (`post_scope_lints`), so this is
    // the one window they can be read in.  A refused file stops here, as any refused file does:
    // its `@EXPECT_ERROR` lines are checked and nothing runs.
    if diagnostics.level() < loft::diagnostics::Level::Error {
        loft::use_analysis::drop_copy_census(&p_data, &mut diagnostics, &path);
        if diagnostics.level() >= loft::diagnostics::Level::Error {
            for l in diagnostics.lines() {
                println!("{l}");
            }
            collected.extend(diagnostics.lines());
            if depth == 0 {
                check_diagnostics(collected, &expected, &exp_errors, &exp_ann_warns, true)?;
            }
            println!("  ok (errors consumed)");
            return Ok(());
        }
    }
    for l in diagnostics.lines() {
        println!("{l}");
    }
    // The post-compile lints join the union too (`warn_dead_stores` and friends run only on a
    // clean parse, so they can only ever appear on the round that reaches here).
    collected.extend(diagnostics.lines());
    if depth == 0 {
        check_diagnostics(collected, &expected, &exp_errors, &exp_ann_warns, true)?;
    }

    // Discover all zero-parameter user functions as entry points.
    let all_fns = entry_point_names(&p_data, start_def);
    assert!(
        !all_fns.is_empty(),
        "no entry-point functions found in {}",
        path
    );
    // If `main` exists, run only `main` (it calls helpers internally).
    // Otherwise run all zero-param functions (fn test_* style).
    let fns = if all_fns.contains(&"main".to_string()) {
        vec!["main".to_string()]
    } else {
        all_fns
    };

    #[cfg(debug_assertions)]
    if allow_dump && std::env::var("LOFT_DUMP").is_ok() {
        let config = LogConfig::from_env();
        let _ = dump_results(entry.clone(), &mut p_data, types, &mut state, &config)?;
    }

    if debug {
        #[cfg(debug_assertions)]
        {
            let config = LogConfig::from_env();
            let mut w = dump_results(entry, &mut p_data, types, &mut state, &config)?;
            for name in &fns {
                state.execute_log(&mut w, name, &config, &p_data)?;
            }
        }
        #[cfg(not(debug_assertions))]
        for name in &fns {
            state.execute(name, &p_data);
        }
    } else {
        // Run each function with catch_unwind so one failure doesn't abort the rest.
        let mut failures: Vec<String> = Vec::new();
        // Part A2 — stack-store free gate (loft#920).  A `BUG (#306)` refusal means the
        // program emitted a whole-store free aimed at the eval-stack store; only the
        // allocator's guard kept it from taking every live frame.  It went to stderr and
        // nothing read it, so `35p-iterator-match.loft` raised one on EVERY run of the
        // suite for as long as it existed and still scored PASS — the refusal keeps the
        // store alive, so nothing the script itself asserts can notice.  Snapshot the
        // process-wide counter here and compare after the run.
        let refusals_before = loft::keys::stack_free_refusals();
        // Did ANY function abort mid-way?  The loop clears `runtime_error` before each
        // function (below), so by the time the leak gate runs it is always `None` and the
        // gate's own reasoning — an aborted run never reaches its scope-frees, so the
        // residue is an abort artifact rather than a leak — silently stops applying to
        // multi-function files.  Remembered here so it applies to them too.
        let mut any_function_faulted = false;
        for name in &fns {
            if std::env::var("LOFT_TEST_VERBOSE").is_ok() {
                eprintln!("  running {path}::{name}");
            }
            let should_fail = file_level_fail || expect_fail.contains(name.as_str());
            // @P369 — the loop reuses ONE `state` across every function in the
            // file; clear the fault flags first so a fault raised by an
            // earlier function does not poison the pass/fail verdict of the
            // ones after it.
            state.database.had_fatal = false;
            state.database.runtime_error = None;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                state.execute(name, &p_data);
                // Drive resume after `yield_frame()` returns control —
                // mirrors the CLI's `while frame_yield { state.resume() }`
                // loop in `src/main.rs:2291-2294`.  Without this, scripts
                // that yield (e.g. `tests/scripts/85-yield-resume.loft`)
                // would only execute the work between fn entry and the
                // first `yield_frame()` call.
                while state.database.frame_yield {
                    state.resume();
                }
                // @P369 — mirror the @P367 CLI-runner fix.  A loft
                // `assert(false)` / `panic()` sets a TYPED runtime error and
                // halts WITHOUT a Rust panic, so `catch_unwind` returns `Ok`
                // and the test would otherwise score as PASSED.  Surface such
                // a fault so the harness can FAIL it (and so an intentional one
                // still satisfies @EXPECT_FAIL).
                //
                // BUT only a genuine test-logic failure (panic / failed
                // assert) fails the test.  A recoverable arithmetic/index fault
                // (div-by-zero, OOB, narrow-cast overflow, …) is the language's
                // designed null-producing behavior — the doc demos
                // (`02-floats`, `17-min-max-clamp`, `23-safety`, …) trigger one
                // on purpose to show the null result and then continue; the
                // script's OWN assertions catch any wrong downstream value, so
                // a recoverable fault must not by itself fail the test.
                let fault_is_failure = state.database.had_fatal
                    && state.database.runtime_error.as_ref().is_none_or(|e| {
                        matches!(
                            e.kind,
                            loft::runtime_error::RuntimeErrorKind::UserPanic { .. }
                                | loft::runtime_error::RuntimeErrorKind::AssertionFailed { .. }
                        )
                    });
                let fault_msg = state
                    .database
                    .runtime_error
                    .as_ref()
                    .map(|e| e.message.clone())
                    .unwrap_or_else(|| "runtime fault".to_string());
                (fault_is_failure, fault_msg)
            }));
            let msg_from = |payload: &Box<dyn std::any::Any + Send>| -> String {
                if let Some(s) = payload.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = payload.downcast_ref::<&str>() {
                    (*s).to_string()
                } else {
                    "unknown panic".to_string()
                }
            };
            match result {
                // @P369 — a typed runtime fault fired (no Rust panic).
                Ok((true, fault_msg)) if should_fail => {
                    any_function_faulted = true;
                    println!("  expected fail {path}::{name} — {fault_msg}");
                }
                Ok((true, fault_msg)) => {
                    any_function_faulted = true;
                    println!("  FAIL {path}::{name} — {fault_msg}");
                    failures.push(format!("{name}: {fault_msg}"));
                }
                Ok((false, _)) if should_fail => {
                    // Bug was fixed — the @EXPECT_FAIL annotation can be removed.
                    println!("  FIXED {path}::{name} (was @EXPECT_FAIL, now passes)");
                }
                Ok((false, _)) => {} // passed as expected
                Err(payload) if should_fail => {
                    println!("  expected fail {path}::{name} — {}", msg_from(&payload));
                }
                Err(payload) => {
                    let msg = msg_from(&payload);
                    println!("  FAIL {path}::{name} — {msg}");
                    failures.push(format!("{name}: {msg}"));
                }
            }
        }
        if !failures.is_empty() {
            return Err(Error::other(format!(
                "{} of {} functions failed in {path}: {}",
                failures.len(),
                fns.len(),
                failures.join("; ")
            )));
        }
        // Part A2 — see the snapshot above.  Reported even when every assertion passed,
        // because passing is exactly what a refused free lets a script keep doing.
        let refused = loft::keys::stack_free_refusals() - refusals_before;
        if refused > 0 {
            return Err(Error::other(format!(
                "{path}: {refused} stack-store free refusal(s) (`BUG (#306)`) — the \
                 program emitted a whole-store free of the eval-stack store and only the \
                 allocator's guard stopped it.  The stderr line names the op and the \
                 nearest source span; the cause is a variable holding a BORROWED record \
                 ref that scope cleanup treated as owning a heap store."
            )));
        }
        // @PLN154 phase 5 — the stack-shadow gate.  The corpus runs IN-PROCESS here, so
        // `main.rs`'s non-zero exit never fires for it; the count is read per script
        // instead, which is also what gives a report the file it belongs to.  Silent and
        // free unless `LOFT_VERIFY_STACK` armed the shadow, and the findings themselves
        // were already printed at their own read sites.
        if stack_verify_gate(shadow_before) {
            return Err(Error::other(format!(
                "{path}: the stack shadow reported an uninitialised, mistyped or stale \
                 slot read — the lines above name the read; run \
                 `LOFT_VERIFY_STACK=1 loft --interpret {path}` to see them alone"
            )));
        }
        // Part B — leak gate: a heap store left unfreed at program exit is a
        // leak (a scope-free regression, hazardous for long-running consumers).
        // `check_store_leaks` prints the by-type warning; then hard-FAIL unless
        // the file is grandfathered in SCRIPTS_LEAK_ALLOW (pre-existing
        // program-end allocations).  This turns the script corpus into a leak
        // regression net — a NEW leak in any non-allowlisted script fails CI.
        //
        // Skip the leak check when a runtime error halted execution mid-main:
        // the dispatch loop short-circuits on `runtime_error`, so the
        // remaining scope-cleanup ops (`OpFreeRef` / `OpFreeText`) never run
        // and the residual stores are NOT a real leak — they are abort
        // artifacts.  Mirrors CLI behaviour in `src/main.rs` (lines 2690-2693).
        // Without this gate, scripts that intentionally probe OOB / div-by-zero
        // mid-main (e.g. `assert(!v[OOB], "OOB is null")`) would always
        // false-positive on the leak gate.
        if state.database.runtime_error.is_none() && !any_function_faulted {
            state.check_store_leaks();
            let leaks = state.collect_store_leaks();
            if !leaks.is_empty() {
                let fname = entry
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if SCRIPTS_LEAK_ALLOW.contains(&fname.as_str()) {
                    println!("  (grandfathered leak — SCRIPTS_LEAK_ALLOW) {path}");
                } else {
                    return Err(Error::other(format!(
                        "{path}: {} store(s) leaked at program exit: {} — fix the \
                         scope-free, or add the file to SCRIPTS_LEAK_ALLOW if it is \
                         an intentional program-end allocation",
                        leaks.len(),
                        leaks.join(", ")
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Write a debug snapshot of a compiled test to `tests/dumps/<filename>.txt`.
///
/// Writes every type definition introduced by the test file (i.e., types beyond
/// those already present in the default library), followed by the full bytecode
/// listing produced by `show_code`.  Returns the open file so the caller can
/// append an execution trace if needed.
#[cfg(debug_assertions)]
fn dump_results(
    entry: PathBuf,
    data: &mut Data,
    types: usize,
    state: &mut State,
    config: &LogConfig,
) -> Result<File, Error> {
    let filename = entry.file_name().unwrap_or_default().to_string_lossy();
    let mut w = File::create(format!("tests/dumps/{filename}.txt"))?;
    for tp in types..state.database.types.len() {
        writeln!(
            &mut w,
            "Type {tp}:{}",
            state.database.show_type(tp as u16, true)
        )?;
    }
    show_code(&mut w, state, data, config)?;
    Ok(w)
}

// @P369 regression — the wrap harness must FAIL a loft test that fires a
// runtime fault (failed assert / panic / OOB) with no @EXPECT_FAIL.  Before
// the fix it scored such a test as PASSED (it only inspected `catch_unwind`,
// not `had_fatal` / `runtime_error`).  Fixtures are written to a temp dir so
// the auto-scanning `loft_suite` (tests/scripts) never runs them standalone.
#[test]
fn p369_silent_runtime_fault_fails_harness() {
    let dir = std::env::temp_dir();

    // Undefended fault, NO @EXPECT_FAIL → must FAIL the harness (run_test Err).
    let bad = dir.join("loft_p369_bad.loft");
    std::fs::write(
        &bad,
        "fn test_p369_silent() { assert(false, \"deliberate @P369 fault\"); }\n",
    )
    .unwrap();
    let r = run_test(bad.clone(), false, false);
    let _ = std::fs::remove_file(&bad);
    assert!(
        r.is_err(),
        "@P369: a failed assert with no @EXPECT_FAIL must FAIL the wrap harness"
    );

    // Control: the SAME fault WITH @EXPECT_FAIL is an expected pass.
    let ok = dir.join("loft_p369_expected.loft");
    std::fs::write(
        &ok,
        "// @EXPECT_FAIL\nfn test_p369_expected() { assert(false, \"deliberate\"); }\n",
    )
    .unwrap();
    let r2 = run_test(ok.clone(), false, false);
    let _ = std::fs::remove_file(&ok);
    assert!(
        r2.is_ok(),
        "@P369: the same fault WITH @EXPECT_FAIL must be scored as an expected pass"
    );
}

/// The `loft test` result line must state WHICH backend produced it.
///
/// `loft test` and `loft test --native` each exercise exactly one backend, so a
/// bare `test result: ok` was identical whether the other backend was clean or
/// had never been compiled once.  A consumer shipped a quarter of their packages
/// with no native coverage at all for as long as those packages had existed,
/// because `loft test` stayed green throughout and nothing said what "green"
/// covered — silence read as coverage.  The scope note therefore rides on the
/// DEFAULT invocation, not behind a flag: the default is the path that was lying.
///
/// Asserted on both invocations, because a note that only appears under
/// `--native` would leave the silent path exactly as it was.
#[test]
fn test_result_states_its_backend_scope() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    let pkg_dir = Path::new("lib/audience_crystal");
    if !pkg_dir.join("tests").is_dir() {
        return Ok(()); // package layout changed; the suite's own runs still cover it
    }

    let out = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, "01-editor-helpers", &[])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let result_line = combined
        .lines()
        .find(|l| l.starts_with("test result:"))
        .unwrap_or_default();
    assert!(
        result_line.contains("ran on the interpreter only"),
        "the default `loft test` result must name the backend it ran on:\n{result_line}"
    );
    assert!(
        result_line.contains("native not exercised"),
        "the default `loft test` result must say the native backend was NOT covered — \
         that omission is the whole defect:\n{result_line}"
    );
    assert!(
        result_line.contains("loft test --native"),
        "the note must carry the command that closes the gap:\n{result_line}"
    );

    // The mirror case: a native-only run must not imply interpreter coverage.
    let out_n = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, "01-editor-helpers", &["--native"])?;
    let combined_n = format!(
        "{}{}",
        String::from_utf8_lossy(&out_n.stdout),
        String::from_utf8_lossy(&out_n.stderr)
    );
    if let Some(line_n) = combined_n.lines().find(|l| l.starts_with("test result:")) {
        assert!(
            line_n.contains("ran on native only"),
            "the `--native` result must name its backend:\n{line_n}"
        );
        assert!(
            line_n.contains("the interpreter not exercised"),
            "a native-only run must not read as full coverage either:\n{line_n}"
        );
    }
    Ok(())
}

/// #631 — `loft test` must EXERCISE admission, and say so.
///
/// The check used to engage only on the run path and via `loft sandbox-check`, so a
/// package could carry a deliberate capability violation and its suite stayed green.
/// A consumer verified exactly that by injecting one; a green suite said nothing about
/// admission, which is easy to mistake for coverage — the same silence-reads-as-coverage
/// shape as the backend scope note above.
///
/// Three states, all asserted: a clean sandboxed package reports the files admission
/// covered, a violating one FAILS, and a policy whose selectors match nothing says so
/// rather than passing quietly (the state that looks identical to real coverage).
#[test]
fn loft_test_runs_admission_and_states_its_scope() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    let tmp = std::env::temp_dir().join(format!("loft_admit_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("tests"))?;

    let policy = |selector: &str| {
        format!(
            "[package]\nname = \"plugpkg\"\nversion = \"0.1.0\"\n\n\
             [sandbox]\nplug = [\"{selector}\"]\n\n\
             [profile.plug]\nallow_libs = [\"code\"]\nmax_input_n = 64\n\
             data_budget = 1048576\n"
        )
    };
    let source = |body: &str| {
        format!(
            "fn total(v: vector<integer>) -> integer {{\n  t = 0;\n  \
             for i in 0..len(v) {{ t += v[i] ?? 0; }}\n  return {body};\n}}\n\
             fn test_total() {{ assert(total([1,2,3]) >= 6, \"sum\"); }}\n"
        )
    };
    let run = || -> std::io::Result<String> {
        let out = std::process::Command::new(loft_bin)
            .current_dir(&tmp)
            .args(["test"])
            .env("LOFT_TIMEOUT", "180")
            // Isolate the spawned `loft test` from the process-global caches under
            // `~/.cache/loft` / `~/.loft`: this suite runs alongside every other
            // test binary on the same runner (the ASan gate builds them into one
            // nextest invocation), and a heavily-loaded shared runner is exactly
            // where cache interference turns a spawn-heavy test flaky.
            .env("LOFT_NO_CACHE", "1")
            .output()?;
        Ok(format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    };
    let result_line = |c: &str| {
        c.lines()
            .find(|l| l.starts_with("test result:"))
            .unwrap_or_default()
            .to_string()
    };

    // 1. Clean sandboxed package — admission runs, passes, and is reported.
    std::fs::write(tmp.join("loft.toml"), policy("fn:total"))?;
    std::fs::write(tmp.join("tests/t_logic.loft"), source("t"))?;
    let clean = result_line(&run()?);
    assert!(
        clean.starts_with("test result: ok."),
        "a clean sandboxed package must pass:\n{clean}"
    );
    assert!(
        clean.contains("admission checked on 1 file"),
        "the result must state that admission covered the file:\n{clean}"
    );

    // 2. The consumer's probe: an injected capability violation must FAIL the suite.
    std::fs::write(
        tmp.join("tests/t_logic.loft"),
        source("t + mtime(\"loft.toml\")"),
    )?;
    let violating = run()?;
    assert!(
        result_line(&violating).starts_with("test result: FAILED."),
        "an ungranted capability must fail the suite — a green run here is the defect:\n{}",
        result_line(&violating)
    );
    assert!(
        violating.contains("Sandbox admission:"),
        "the failure must name admission as the cause:\n{violating}"
    );

    // 3. A policy that designates NOTHING must say so; passing quietly is
    //    indistinguishable from real coverage, which is the whole complaint.
    std::fs::write(tmp.join("loft.toml"), policy("fn:no_such_function"))?;
    std::fs::write(tmp.join("tests/t_logic.loft"), source("t"))?;
    let empty = result_line(&run()?);
    assert!(
        empty.contains("designated nothing here"),
        "a policy matching no code must be reported, not silently passed:\n{empty}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}
