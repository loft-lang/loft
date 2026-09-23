// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F89 — Test runner (fn test_*, loft --tests)

//! Test runner: discover and run callable functions in `.loft` files.

#![allow(unused_imports)] // Module used from main(), not from test builds.

use crate::compile;
use crate::data::{Data, DefType, Type};
use crate::generation;
use crate::log_config::LogConfig;
use crate::logger;
use crate::native_utils;
use crate::parser::Parser;
use crate::scopes;
use crate::state::State;
use std::collections::HashSet;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// RAII process-cwd guard for the duration of one program's execution.
///
/// loft's `file()` resolves a relative path against `source_dir`
/// (`Stores::resolve_path`), but a native crate's raw `std::fs` resolves against
/// the process cwd — so the two diverge when cwd ≠ source_dir (e.g. `loft test`
/// runs from the package root while a test file lives in `tests/`, breaking
/// imaging's `load_png`/`save_png`).  `enter_source_dir` chdir's to `source_dir`
/// for the run so native I/O anchors where loft's does; the guard restores the
/// original cwd on drop, so the NEXT serially-run test file's parse/compile is
/// unaffected.  Gated on `program_relative` so a `#cwd` program (which anchors
/// loft I/O at the cwd) keeps native I/O at the cwd too.
struct CwdGuard(Option<std::path::PathBuf>);
impl Drop for CwdGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.0.take() {
            let _ = std::env::set_current_dir(prev);
        }
    }
}
/// @PLN86 / #631 — the `[sandbox]` policy governing `file`, or `None` when no
/// package above it declares one.
///
/// The run path reads `loft.toml` from the directory beside the file it loads, but
/// a test file lives in `tests/` while the code it exercises lives in `src/`, so the
/// policy is at the package ROOT.  Walking up finds it from either place; without
/// that walk `loft test` would keep silently skipping admission for every package
/// laid out normally, which is the whole defect.
fn sandbox_policy_for(file: &str) -> Option<loft::sandbox::SandboxConfig> {
    let mut dir = std::path::Path::new(file).parent()?;
    for _ in 0..4 {
        if let Ok(content) = std::fs::read_to_string(dir.join("loft.toml")) {
            let cfg = loft::sandbox::parse_sandbox_config(&content);
            if cfg.is_active() {
                return Some(cfg);
            }
        }
        dir = dir.parent()?;
    }
    None
}

/// The root of the package a test file belongs to — the nearest ancestor holding a
/// `loft.toml`.  A test lives in `tests/` while the code it drives lives in `src/`, so
/// the walk starts beside the file and climbs.  `None` for a loose script with no
/// package around it.
fn package_root_for(file: &str) -> Option<std::path::PathBuf> {
    let mut dir = std::path::Path::new(file).parent()?;
    for _ in 0..4 {
        if dir.join("loft.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
    None
}

/// The path to report a definition under for coverage, or `None` when the package
/// under test is not answerable for it.
///
/// Answerable means: inside this package's own directory.  That excludes the stdlib,
/// dependencies (whether resolved from the registry cache, a lib dir, or a sibling
/// directory in the same checkout), and the test file itself — its `fn test_*` are the
/// drivers, which the runner already reports on. Charging a package for a dependency
/// would make its number depend on how much of that dependency it happens to touch,
/// which says nothing about the package's own tests.
///
/// Returns the path relative to the package root, so the report reads
/// `src/regex.loft:30` rather than an absolute path nobody can scan.
fn coverage_path(src: &str, test_file: &str, root: Option<&std::path::Path>) -> Option<String> {
    if src.is_empty() || crate::portable_path::is_stdlib_source(src) {
        return None;
    }
    let abs = crate::portable_path::try_plain_canonical(std::path::Path::new(src))?;
    if let Some(t) = crate::portable_path::try_plain_canonical(std::path::Path::new(test_file))
        && abs == t
    {
        return None;
    }
    let root = root?;
    let root = crate::portable_path::try_plain_canonical(root)?;
    let rel = abs.strip_prefix(&root).ok()?;
    // This string is a REPORT — something a reader copies into an editor, and something
    // a test asserts on — not a path anything opens, so it must read the same on every
    // platform.  `to_string_lossy()` alone hands back the native separator, which made
    // the Windows leg print `src\pos.loft` against a contract (and a `loft.toml`
    // `entry = "src/<name>.loft"`) that says `src/pos.loft`.
    Some(crate::portable_path::portable(rel))
}

/// loft#925 — what is known about one group of test files: those that open with
/// exactly the same `use` region and search the same library path.
enum BaseSlot {
    /// Exactly one file has asked so far, and it parsed for itself.  A base is
    /// only worth building once a second file wants the same libraries.
    Once,
    /// The group's shared parse, or `None` when it could not be built.
    ///
    /// Boxed because a `TestBase` owns a whole `Parser`, and this enum sits in a
    /// map with one entry per group — most of them `Once`.
    Shared(Option<Box<TestBase>>),
}

/// loft#925 — a completed parse of the stdlib plus one group's libraries,
/// shared by every test file that opens with exactly that `use` region.
struct TestBase {
    parser: Parser,
    /// Definition count after the stdlib and before the libraries — the boundary
    /// a per-file parse would have captured for itself, recorded here because a
    /// seeded file never loads the stdlib on its own.
    stdlib_defs: u32,
    /// What the library parse reported.  The per-file parse no longer sees the
    /// library sources, so these are carried forward and reported against every
    /// file in the group — which is where they appeared before.
    diagnostics: Vec<String>,
}

/// loft#925 — the leading `use` region of a test file, verbatim, or `None` when
/// this file has to be parsed the ordinary way.
///
/// Verbatim rather than interpreted: the text collected here becomes the base's
/// whole source, so the parser is the one deciding what those lines mean, and a
/// group key of the same text is a group whose libraries are the same set by
/// construction.  Re-deriving the meaning here would be a second answer to a
/// question the parser already answers.
///
/// A leading file directive (`#cwd`) is part of the region rather than a reason
/// to give up.  It has to be — every one of the 81 test files in the consumer
/// that reported this opens with one, so refusing it would have made the whole
/// change do nothing for the case that motivated it.  Carrying it VERBATIM is
/// also what keeps the "let the parser decide" rule: whatever a directive means,
/// the base means the same, where deciding here that `#cwd` cannot matter would
/// be a judgement to re-check every time a directive is added.  A directive the
/// parser rejects makes the base error out, and an erroring base is refused.
///
/// `None` for everything not plainly an optional directive followed by complete
/// `use` statements — a `use` sharing its line with code, an unterminated
/// statement, a directive after a `use`, or no `use` at all.  A file with no
/// `use` may still load libraries through the auto-`use` pre-scan, which fires
/// only when a file writes none, so that is not an empty group: it is a file
/// that must parse for itself.
fn leading_use_region(source: &str) -> Option<String> {
    let mut region = String::new();
    let mut pending = String::new();
    let mut saw_use = false;
    for raw in source.lines() {
        let line = raw.trim();
        if !pending.is_empty() {
            // Continuation of a `use` that has not reached its `;` yet.
        } else if line.is_empty() || line.starts_with("//") {
            continue;
        } else if line.starts_with('#') {
            // The parser reads the directive at the very top of the file, before
            // anything else, so anywhere else it is not one.
            if saw_use || !region.is_empty() {
                return None;
            }
            region.push_str(line);
            region.push('\n');
            continue;
        } else if !(line == "use" || line.starts_with("use ") || line.starts_with("use\t")) {
            break; // first definition — the use region ends here
        }
        if !pending.is_empty() {
            pending.push(' ');
        }
        pending.push_str(line);
        let Some(end) = pending.find(';') else {
            continue; // keep reading: `use lib::(a,\n b);`
        };
        // Nothing but a comment may follow the `;` — otherwise the line holds
        // code the base must not contain.
        let tail = pending[end + 1..].trim();
        if !tail.is_empty() && !tail.starts_with("//") {
            return None;
        }
        region.push_str(&pending[..=end]);
        region.push('\n');
        pending.clear();
        saw_use = true;
    }
    if !pending.is_empty() {
        return None; // ran out of file mid-statement
    }
    saw_use.then_some(region)
}

/// loft#925 — parse `use_region` on its own, so the libraries it names are
/// parsed once for the whole group instead of once per test file.
///
/// `base_file` is a path in the test files' own directory that is never read:
/// it is what decides the source directory and the owning package, and both
/// must be the ones a test file beside it would get.
///
/// `None` — parse the group's files the ordinary way — whenever the base cannot
/// stand in for that: no stdlib, a panic, or any error-level diagnostic.  An
/// error belongs to the file the reader is being shown, so it is left to be
/// re-emitted by that file's own parse rather than replayed out of a base.
fn build_test_base(
    default_dir: &str,
    lib_dirs: &[String],
    base_file: &str,
    use_region: &str,
) -> Option<Box<TestBase>> {
    let built = build_test_base_inner(default_dir, lib_dirs, base_file, use_region);
    // `LOFT_TEST_BASE_REPORT=1` — say whether the sharing engaged, and for which
    // libraries.  Off by default and on stderr: a run has nothing to report here,
    // and a suite that got slower is the only reason to ask.  It is also what
    // keeps the equivalence guard honest — a sharing that quietly stopped
    // happening would leave that guard passing against itself.
    if std::env::var("LOFT_TEST_BASE_REPORT").is_ok_and(|v| v == "1" || v == "true") {
        let libs = use_region.replace('\n', " ");
        let libs = libs.trim();
        if built.is_some() {
            eprintln!("loft: test base shared — {libs}");
        } else {
            eprintln!("loft: test base refused — {libs}");
        }
    }
    built
}

/// The `default/` directory holding the stdlib, below the directory `--path` names.
///
/// Joined with a path separator rather than concatenated (loft#1112).  The two
/// spellings of the same directory must mean the same thing: `project_dir()` hands
/// back a trailing-separator path, so a plain `+ "default"` happened to work for it,
/// while `--path <dir>` arrives verbatim from the command line and yielded
/// `<dir>default` — a directory that does not exist, reported as *"cannot load
/// default library"* against a file that passes without the flag.  `main.rs` joins
/// the same way for an ordinary run (@P363); this is the test runner's half.
fn stdlib_dir_of(default_dir: &str) -> String {
    std::path::Path::new(default_dir)
        .join("default")
        .to_string_lossy()
        .into_owned()
}

fn build_test_base_inner(
    default_dir: &str,
    lib_dirs: &[String],
    base_file: &str,
    use_region: &str,
) -> Option<Box<TestBase>> {
    let mut p = Parser::new();
    p.lib_dirs = lib_dirs.to_vec();
    let stdlib_dir = stdlib_dir_of(default_dir);
    if !loft::startup_cache::warm_load_stdlib(&mut p, &stdlib_dir)
        && p.parse_dir(&stdlib_dir, true, false).is_err()
    {
        return None;
    }
    let stdlib_defs = p.data.definitions();
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        p.parse_as(base_file, use_region, false);
    }));
    if parsed.is_err() {
        return None;
    }
    if p.diagnostics.level() >= loft::diagnostics::Level::Error {
        return None;
    }
    // Only what the LIBRARY SOURCES said travels.  A diagnostic positioned at the
    // base file is one about the use region itself — a module-name clash, say —
    // and every file in the group writes that same region and re-emits it at its
    // own line.  Carrying it too would print it twice, the second time against a
    // file name that exists nowhere.
    let diagnostics = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.file != base_file)
        .map(loft::diagnostics::DiagEntry::to_string_compact)
        .collect();
    Some(Box::new(TestBase {
        parser: p,
        stdlib_defs,
        diagnostics,
    }))
}

fn enter_source_dir(source_dir: &str, program_relative: bool) -> CwdGuard {
    if program_relative
        && !source_dir.is_empty()
        && let Ok(prev) = std::env::current_dir()
        && std::env::set_current_dir(source_dir).is_ok()
    {
        return CwdGuard(Some(prev));
    }
    CwdGuard(None)
}

/// Run all zero-parameter functions in `.loft` files under `root_dir` as tests.
/// Supports `@ARGS`, `@EXPECT_ERROR`, and `@EXPECT_FAIL` file annotations.
/// Returns 0 if all pass, 1 if any fail.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub(crate) fn run_tests(
    default_dir: &str,
    root_dir: &str,
    no_warnings: bool,
    deny_warnings: bool,
    lib_dirs: &[String],
    project: Option<&str>,
    native_mode: bool,
    extra_native_libs: &[String],
) -> i32 {
    use crate::data::DefType;
    use std::collections::BTreeMap;

    // A ceiling on the store heap, for test runs only.  A test that wants tens of
    // gigabytes is a bug either way, and a corrupted length does not always end in a
    // bad dereference — often it ends in an allocation, which no time bound catches
    // and which the kernel's OOM killer answers by killing something, not necessarily
    // the culprit.  Crossing the ceiling stops the run at the growth that crossed it
    // and says which type was growing and how the rest of the heap was distributed.
    // `LOFT_MEMORY_LIMIT=0` removes it; ordinary `loft prog.loft` runs are never
    // capped, because loft is unbounded by default and a real program may want the
    // whole machine.
    loft::store_budget::apply_env_limit(loft::store_budget::DEFAULT_TEST_LIMIT);

    struct FileResult {
        tests: Vec<(String, bool, Option<String>)>, // (fn_name, passed, fail_msg)
        warnings: Vec<String>,
        /// `Level::Advice` lines, kept SEPARATE from `warnings` so `--deny-warnings`
        /// cannot fail on them: advice reports correct code, and a library must not be
        /// unable to pass its own CI because loft gained a deprecation.  They still
        /// satisfy `@EXPECT_WARNING`, because a test asserting a diagnostic fires is
        /// asking whether it FIRED, not which tier it landed in.
        advice: Vec<String>,
        errors: Vec<String>,
    }

    // ── Annotations parsed from `// @` header comments ──────────────
    #[derive(Default)]
    struct Annotations {
        /// File-level `@IGNORE` — skip the entire file.
        ignore_file: bool,
        /// Per-function `@IGNORE`: `fn_name` → true.
        ignore_fn: std::collections::HashSet<String>,
        /// File-level `@EXPECT_ERROR` substrings — every one must match an error.
        expect_errors: Vec<String>,
        /// Per-function `@EXPECT_ERROR`: `fn_name` → required substrings.
        ///
        /// Ordered, like the two maps below it, because the run REPORTS the
        /// function names it satisfied — and a hash map's iteration order is
        /// randomised per process, so the same green run printed its list in a
        /// different order every time.  A report a reader diffs across runs has
        /// to be stable.
        expect_errors_fn: std::collections::BTreeMap<String, Vec<String>>,
        /// File-level `@EXPECT_WARNING` substrings — all must appear in warnings.
        expect_warnings: Vec<String>,
        /// Per-function `@EXPECT_WARNING`: `fn_name` → required substrings.
        expect_warnings_fn: std::collections::BTreeMap<String, Vec<String>>,
        /// File-level `@EXPECT_FAIL` substrings — every function is expected to
        /// panic with a message containing one of these.
        expect_fail_file: Vec<String>,
        /// Per-function `@EXPECT_FAIL`: `fn_name` → required substrings.
        expect_fail_fn: std::collections::BTreeMap<String, Vec<String>>,
        /// Extra --lib dirs from @ARGS.
        extra_lib_dirs: Vec<String>,
        /// --project from @ARGS.
        project: Option<String>,
        /// --production from @ARGS.
        production: bool,
        /// --log-conf from @ARGS.
        log_conf: Option<String>,
        /// Positional arguments from @ARGS (passed as argv).
        user_args: Vec<String>,
    }

    /// Scan the raw source for `// @` annotations.  Only comments before the
    /// first non-comment, non-blank line (or before a `fn`/`struct`/`enum`
    /// definition) are considered file-level.  A `// @EXPECT_FAIL` on the
    /// line directly before a `fn <name>` binds to that function.
    fn parse_annotations(src: &str) -> Annotations {
        let mut ann = Annotations::default();
        // Pending annotations not yet bound to a function.
        let mut pending_fail: Vec<String> = Vec::new();
        let mut pending_error: Vec<String> = Vec::new();
        let mut pending_warning: Vec<String> = Vec::new();
        let mut pending_ignore = false;
        // True until we see the first fn/struct/enum definition.
        let mut in_header = true;

        for line in src.lines() {
            let trimmed = line.trim();

            // Check for fn definition — bind pending annotations.
            if trimmed.starts_with("fn ") {
                in_header = false;
                if let Some(name) = trimmed
                    .strip_prefix("fn ")
                    .and_then(|s| s.split(&['(', ' ', '{'][..]).next())
                {
                    let name = name.trim();
                    if !name.is_empty() {
                        if !pending_fail.is_empty() {
                            ann.expect_fail_fn
                                .entry(name.to_string())
                                .or_default()
                                .append(&mut pending_fail);
                        }
                        if !pending_error.is_empty() {
                            ann.expect_errors_fn
                                .entry(name.to_string())
                                .or_default()
                                .append(&mut pending_error);
                        }
                        if !pending_warning.is_empty() {
                            ann.expect_warnings_fn
                                .entry(name.to_string())
                                .or_default()
                                .append(&mut pending_warning);
                        }
                        if pending_ignore {
                            ann.ignore_fn.insert(name.to_string());
                        }
                    }
                }
                pending_fail.clear();
                pending_error.clear();
                pending_warning.clear();
                pending_ignore = false;
                continue;
            }

            // Struct/enum definitions end the header.  A `struct` or `enum` cannot BE a
            // test, so an annotation in front of one has no function to bind to — but it
            // still describes a diagnostic the file must produce, so it becomes file-level
            // rather than being discarded.  It used to be discarded: six `@EXPECT_ERROR`s
            // in `102b-pass1-expected-errors.loft` sit in front of the deliberately-invalid
            // `struct integer` / `enum hash` declarations they describe, and not one of them
            // was ever checked.
            if trimmed.starts_with("struct ") || trimmed.starts_with("enum ") {
                in_header = false;
                pending_ignore = false;
                ann.expect_fail_file.append(&mut pending_fail);
                ann.expect_errors.append(&mut pending_error);
                ann.expect_warnings.append(&mut pending_warning);
                continue;
            }

            // Only process // comments.  Blank lines are preserved —
            // they must NOT clear pending annotations so that:
            //   // @EXPECT_ERROR: msg
            //
            //   fn test_foo() { ... }
            // still binds the annotation to test_foo.
            let Some(comment) = trimmed.strip_prefix("//") else {
                if !trimmed.is_empty() {
                    // Non-comment, non-blank line — no function follows, so promote rather
                    // than drop, for the reason given at the struct/enum arm above.  This is
                    // the arm that reaches an annotation written INSIDE a body:
                    // `persist-bind-field-store-757.loft` carries one there, and it was inert
                    // — replacing its text with a warning that exists nowhere left the file
                    // passing exactly as before.
                    ann.expect_fail_file.append(&mut pending_fail);
                    ann.expect_errors.append(&mut pending_error);
                    ann.expect_warnings.append(&mut pending_warning);
                    pending_ignore = false;
                }
                continue;
            };
            let comment = comment.trim();

            if let Some(rest) = comment.strip_prefix("@EXPECT_ERROR:") {
                let sub = rest.trim();
                if !sub.is_empty() {
                    if in_header {
                        ann.expect_errors.push(sub.to_string());
                    } else {
                        pending_error.push(sub.to_string());
                    }
                }
            } else if let Some(rest) = comment.strip_prefix("@EXPECT_WARNING:") {
                let sub = rest.trim();
                if !sub.is_empty() {
                    if in_header {
                        ann.expect_warnings.push(sub.to_string());
                    } else {
                        pending_warning.push(sub.to_string());
                    }
                }
            } else if let Some(rest) = comment.strip_prefix("@EXPECT_FAIL:") {
                let sub = rest.trim();
                if !sub.is_empty() {
                    if in_header {
                        ann.expect_fail_file.push(sub.to_string());
                    } else {
                        pending_fail.push(sub.to_string());
                    }
                }
            } else if comment.starts_with("@IGNORE") {
                if in_header {
                    ann.ignore_file = true;
                } else {
                    pending_ignore = true;
                }
            } else if let Some(rest) = comment.strip_prefix("@ARGS:") {
                parse_args_annotation(rest.trim(), &mut ann);
            }
        }
        // Any pending annotations not followed by a fn → file-level.
        ann.expect_fail_file.append(&mut pending_fail);
        ann.expect_errors.append(&mut pending_error);
        ann.expect_warnings.append(&mut pending_warning);
        if pending_ignore {
            ann.ignore_file = true;
        }
        ann
    }

    /// Parse the token list after `@ARGS:` using the same flag convention as
    /// the main CLI.  Unknown flags are silently ignored so that future flags
    /// don't break old test files.
    fn parse_args_annotation(s: &str, ann: &mut Annotations) {
        let tokens: Vec<&str> = s.split_whitespace().collect();
        let mut i = 0;
        while i < tokens.len() {
            let t = tokens[i];
            i += 1;
            if t == "--lib" {
                if let Some(&dir) = tokens.get(i) {
                    ann.extra_lib_dirs.push(dir.to_string());
                    i += 1;
                }
            } else if t == "--project" {
                if let Some(&dir) = tokens.get(i) {
                    ann.project = Some(dir.to_string());
                    i += 1;
                }
            } else if t == "--production" {
                ann.production = true;
            } else if t == "--log-conf" {
                if let Some(&path) = tokens.get(i) {
                    ann.log_conf = Some(path.to_string());
                    i += 1;
                }
            } else if t.starts_with('-') {
                // Unknown flag — skip (and consume a value arg if present).
                if tokens.get(i).is_some_and(|s| !s.starts_with('-')) {
                    i += 1;
                }
            } else {
                // Positional argument — this and all remaining tokens are user args.
                ann.user_args.push(t.to_string());
                for &rest in &tokens[i..] {
                    ann.user_args.push(rest.to_string());
                }
                break;
            }
        }
    }

    // Recursively collect .loft files grouped by directory.
    fn collect_loft_files(
        dir: &std::path::Path,
        out: &mut BTreeMap<String, Vec<std::path::PathBuf>>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut files = Vec::new();
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip hidden directories and .loft artifact dirs
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if !name.starts_with('.') {
                    subdirs.push(path);
                }
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            {
                files.push(path);
            }
        }
        files.sort();
        if !files.is_empty() {
            let dir_key = dir.to_string_lossy().to_string();
            out.insert(dir_key, files);
        }
        subdirs.sort();
        for sub in subdirs {
            collect_loft_files(&sub, out);
        }
    }

    fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
        if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else {
            "unknown panic".to_string()
        }
    }

    /// Why one native test run failed: the first `error:` line the program wrote (an
    /// assertion names itself there, as it does on the interpreter), else how it ended.
    fn native_failure(status: std::process::ExitStatus, stderr: &str) -> String {
        if let Some(line) = stderr.lines().find_map(|l| l.strip_prefix("error: ")) {
            return line.trim().to_string();
        }
        if let Some(code) = status.code() {
            return format!("native run failed (exit {code})");
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(sig) = status.signal() {
                return format!("native run killed by signal {sig}");
            }
        }
        "native run failed".to_string()
    }

    /// The declared substrings that no produced error contains.
    ///
    /// This is the whole of TESTING.md's **"Every expectation must match"**: an
    /// `@EXPECT_ERROR` names a diagnostic the file owes, so an empty result is the only
    /// passing answer.  Both the file-level and the per-function checks ask exactly this
    /// question, and it lives here once so the two cannot answer it differently — the
    /// per-function site used to settle it with "does the file have SOME error?", an
    /// existential standing in for a universal, which credited every annotation in a file
    /// that produced one error anywhere (loft#1261).
    fn unmatched_expect<'a>(subs: &'a [String], errors: &[String]) -> Vec<&'a str> {
        subs.iter()
            .filter(|sub| !errors.iter().any(|e| e.contains(sub.as_str())))
            .map(String::as_str)
            .collect()
    }

    /// Check whether `msg` satisfies the expected-fail substrings for `fn_name`.
    /// Returns true when the panic message contains at least one required
    /// substring (file-level or per-function).
    fn matches_expect_fail(ann: &Annotations, fn_name: &str, msg: &str) -> bool {
        // Per-function annotations take priority.
        if let Some(subs) = ann.expect_fail_fn.get(fn_name) {
            return subs.iter().any(|s| msg.contains(s.as_str()));
        }
        // Fall back to file-level.
        if !ann.expect_fail_file.is_empty() {
            return ann
                .expect_fail_file
                .iter()
                .any(|s| msg.contains(s.as_str()));
        }
        false
    }

    // Suppress the default panic hook output ("thread 'main' panicked at ...").
    // All panics inside the runner are caught by catch_unwind; we extract the
    // message from the payload and report it ourselves in the test summary.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    // Parse optional function filter: "file.loft::name" or "file.loft::{a,b}".
    let (path_part, fn_filter): (&str, Option<Vec<String>>) = if let Some(pos) = root_dir.find("::")
    {
        let raw = &root_dir[pos + 2..];
        let names: Vec<String> = if raw.starts_with('{') && raw.ends_with('}') {
            raw[1..raw.len() - 1]
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        } else {
            vec![raw.to_string()]
        };
        (&root_dir[..pos], Some(names))
    } else {
        (root_dir, None)
    };

    let root = std::path::Path::new(path_part);
    let mut dirs: BTreeMap<String, Vec<std::path::PathBuf>> = BTreeMap::new();
    if root.is_file() {
        // Single file mode: run tests in just this file.
        let dir_key = root
            .parent()
            .map_or(".".to_string(), |p| p.to_string_lossy().to_string());
        dirs.insert(dir_key, vec![root.to_path_buf()]);
    } else if root.is_dir() {
        collect_loft_files(root, &mut dirs);
    } else {
        std::panic::set_hook(prev_hook);
        println!("loft: --tests path '{path_part}' does not exist");
        return 1;
    }

    if dirs.is_empty() {
        std::panic::set_hook(prev_hook);
        println!("loft: no .loft files found in '{path_part}'");
        return 1;
    }

    // Build the project lib path once, if --project was supplied on the CLI.
    let project_lib: Option<String> = project.map(|proj| {
        std::path::Path::new(proj)
            .join("lib")
            .to_str()
            .unwrap_or("")
            .to_string()
    });

    // In native mode, ensure libloft.rlib exists and is up to date.
    // `cargo run --bin loft` rebuilds the binary but may skip the library
    // target, leaving native tests linking against stale code.
    // Detect this by comparing source mtimes against the rlib and rebuild
    // automatically when needed.
    if native_mode {
        native_utils::ensure_rlib_fresh();
        if native_utils::loft_lib_dir().is_none() {
            std::panic::set_hook(prev_hook);
            println!(
                "loft: --native requires libloft.rlib; \
                 run `cargo build --lib` first"
            );
            return 1;
        }
    }

    // loft#860 — `--native` compiles each test to Rust, so there is no dispatch loop
    // to sample and no loft call stack to sample it over.  Said once, up front, because
    // the alternative is a run that accepts the variable and ends without a report —
    // which reads as "the profiler found nothing", the one thing an instrument must
    // never say when it did not run.
    if native_mode
        && (std::env::var_os("LOFT_PROFILE").is_some()
            || std::env::var_os("LOFT_ALLOC_PATHS").is_some())
    {
        eprintln!(
            "loft: the loft-level profiler is interpreter-only — these tests run --native, \
             so nothing\n  will be sampled. Drop --native to profile them, or use \
             `make profile PROFILE_FLAGS=--engine`\n  to profile the generated binary with perf."
        );
    }

    let mut total_pass = 0u32;
    let mut total_fail = 0u32;
    let mut total_files = 0u32;
    // Tests counted as PASSED on the reported backend that never actually ran on
    // it (`@EXPECT_FAIL` / `@IGNORE` under `--native`, or a file with no
    // native-runnable fn).  Reported so a green count cannot stand in for
    // coverage it does not have.
    let mut total_skipped = 0u32;
    // Files whose package declares a `[sandbox]` policy AND that designate sandboxed
    // code — the ones admission actually covered.  Reported so a green run cannot be
    // read as admission coverage it never had (#631).
    let mut sandbox_checked_files = 0u32;
    // A `[sandbox]` policy was found for at least one file, whether or not it
    // designated anything.  Kept apart from `sandbox_checked_files` so a policy
    // that matches NOTHING reports as its own state — that is the silent case, and
    // calling it "no policy" would hide exactly what needs saying.
    let mut sandbox_policy_seen = false;
    // Function coverage, accumulated across every test file by a STABLE identity
    // (file, line, name) rather than by `d_nr`: each test file gets its own parser, so
    // the same function has a different index in each one.  A function entered by ANY
    // test in the run counts as reached.  Reported, never gated — a library is written
    // before its consumers exist, so a coverage bar would punish exactly the case the
    // package system is meant to support.
    let mut coverage: BTreeMap<(String, u32, String), bool> = BTreeMap::new();
    let mut dir_summaries: Vec<(String, u32, u32)> = Vec::new(); // (dir, pass, fail)
    // loft#860 — the loft-level profile, merged across every test in the run.
    //
    // A suite is usually the biggest interpreted workload a project owns, and the one
    // whose time everyone notices; before this it was the one workload `LOFT_PROFILE`
    // could not see, because the sampler was armed at exactly one call site on the
    // program path.  Each test gets a fresh `State` *and* a fresh `Data`, so the
    // samples are resolved to `(function, file:line)` per test and merged on those —
    // see `Totals`.  A `RefCell` because the arming and the folding both happen inside
    // the per-test `catch_unwind` closure.
    let profile = std::cell::RefCell::new(loft::profiler::Totals::default());

    // loft#925 — the library parse, shared across the test files that ask for
    // exactly the same libraries.
    //
    // A suite builds one parser per test file, and the `use`d library was loaded
    // into each of them from source — twice over, since both parse passes re-run
    // the use region.  That cost is proportional to the library and paid once per
    // file, so a suite pays the PRODUCT of its size and its library's: dryopea's
    // 67 files spent ~31 s of a ~145 s run re-compiling one unchanged library.
    //
    // Keyed on (directory, lib search path, use region) — every input the library
    // parse reads — so a file only ever starts from a base holding the libraries
    // it actually named.  A base carrying MORE than that would resolve names the
    // file cannot see, which is a silently wrong compile, not a slow one.
    //
    // `Shared(None)` records a group whose base could not be built; those files parse
    // the ordinary way, so a refusal costs speed and nothing else.  `LOFT_NO_TEST_BASE=1`
    // refuses every group — the opt-out half of an A/B on one binary.
    let no_base = std::env::var("LOFT_NO_TEST_BASE").is_ok_and(|v| v == "1" || v == "true");

    for (dir_path, files) in &dirs {
        let mut dir_pass = 0u32;
        let mut dir_fail = 0u32;
        // Per DIRECTORY, and dropped with it.  A base holds a whole parsed program,
        // and it could never have been reused across directories anyway: the base
        // file has to sit beside the test files, because that is what decides the
        // source directory and the owning package a `use` resolves against.
        let mut bases: BTreeMap<(Vec<String>, String), BaseSlot> = BTreeMap::new();

        for file_path in files {
            let abs_file = crate::portable_path::plain_canonical(file_path)
                .to_str()
                .unwrap_or("")
                .to_string();
            // `/` on every platform.  This name is what every `ok` / `FAIL` line below
            // prints, so it is read by someone who did not necessarily run it — in a CI log,
            // in a bug report, or in the reference pages, whose transcripts `tests/doc_commands.rs`
            // checks literally.  On Windows the host separator made the runner print
            // `greeter\tests\greet.loft` where the page shows `greeter/tests/greet.loft`, and the
            // documented output did not match the real one.
            let display_name = crate::portable_path::portable(file_path);

            // Read the raw source to extract annotations before parsing.
            let source = match std::fs::read_to_string(file_path) {
                Ok(s) => s,
                Err(e) => {
                    println!("  FAIL  {display_name}  (cannot read: {e})");
                    dir_fail += 1;
                    total_files += 1;
                    continue;
                }
            };
            let ann = parse_annotations(&source);
            if ann.ignore_file {
                continue; // silently skip ignored files
            }
            let has_expect_error = !ann.expect_errors.is_empty();

            // Build parser with CLI lib_dirs + @ARGS lib dirs.
            let mut p = Parser::new();
            p.lib_dirs = lib_dirs.to_vec();
            if let Some(ref pl) = project_lib {
                p.lib_dirs.insert(0, pl.clone());
            }
            for extra in &ann.extra_lib_dirs {
                p.lib_dirs.push(extra.clone());
            }
            if let Some(ref proj) = ann.project {
                p.lib_dirs.insert(
                    0,
                    std::path::Path::new(proj)
                        .join("lib")
                        .to_str()
                        .unwrap_or("")
                        .to_string(),
                );
            }
            // @PLN86 / #631 — apply the package's `[sandbox]` policy so admission runs
            // here too.  It used to engage only on the run path and `loft sandbox-check`,
            // so a suite stayed green with a deliberate capability violation injected —
            // the same silence-reads-as-coverage shape as the backend scope below.
            // Designation must be set BEFORE parsing: `def_sandbox` forms during the
            // parse, and admission reads what the parse recorded.
            let sandboxed = if let Some(policy) = sandbox_policy_for(&abs_file) {
                sandbox_policy_seen = true;
                p.set_sandbox_config(policy);
                true
            } else {
                false
            };
            // loft#925 — start from the group's shared parse of stdlib + libraries
            // when there is one.  Skipped under a `[sandbox]` policy: admission reads
            // what the PARSE recorded about designated functions, and those side maps
            // belong to the parse that produced them, so a shared base would quietly
            // stop checking the library — the one failure mode worse than the 31 s.
            let mut base_diagnostics: Vec<String> = Vec::new();
            let mut seeded: Option<u32> = None;
            if !no_base
                && !sandboxed
                && let Some(region) = leading_use_region(&source)
            {
                let key = (p.lib_dirs.clone(), region);
                match bases.entry(key) {
                    // First file with this use region: parse it the ordinary way.
                    // A base only pays off when it is shared, and building one here
                    // would DOUBLE the cost of `loft test <one-file>` — the tight
                    // inner loop of development, and a group of one by definition.
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert(BaseSlot::Once);
                    }
                    // A second file wants the same libraries, so the parse is worth
                    // sharing: build it now and hand it to this file and every later
                    // one.  The group pays one ordinary parse plus one base.
                    std::collections::btree_map::Entry::Occupied(mut slot) => {
                        if matches!(slot.get(), BaseSlot::Once) {
                            let base_file = std::path::Path::new(&abs_file)
                                .with_file_name("__loft_test_base.loft")
                                .to_string_lossy()
                                .into_owned();
                            let (libs, region) = slot.key();
                            let built = build_test_base(default_dir, libs, &base_file, region);
                            slot.insert(BaseSlot::Shared(built));
                        }
                        if let BaseSlot::Shared(Some(base)) = slot.get() {
                            p.seed_from(&base.parser);
                            base_diagnostics.clone_from(&base.diagnostics);
                            seeded = Some(base.stdlib_defs);
                        }
                    }
                }
            }
            // loft#925 — warm-load the stdlib instead of re-parsing `default/` for
            // every test file.  A suite builds ONE parser per file (each test file is
            // its own program, and must stay that way — a shared parser would let one
            // file's definitions leak into the next), so the stdlib parse was paid
            // once per file for a directory that cannot have changed between them.
            // The bundle is the same one `loft <program>` already loads, keyed on the
            // stdlib directory, so this reuses a cache the run has usually warmed
            // already rather than adding one.  A miss falls through to the cold parse
            // exactly as before.
            //
            // A seeded file skips it outright: the base holds the same stdlib, so
            // loading the bundle here would decode it only to have `seed_from`
            // discard it — which is most of what a seeded file still paid.
            let start_def = if let Some(stdlib_defs) = seeded {
                stdlib_defs
            } else {
                let stdlib_dir = stdlib_dir_of(default_dir);
                if !loft::startup_cache::warm_load_stdlib(&mut p, &stdlib_dir) {
                    if p.parse_dir(&stdlib_dir, true, false).is_err() {
                        println!("  FAIL  {display_name}  (cannot load default library)");
                        dir_fail += 1;
                        total_files += 1;
                        continue;
                    }
                    loft::startup_cache::save_stdlib_cache(&p, &stdlib_dir);
                }
                // The stdlib boundary.  Everything past it belongs to the program
                // under test: the native codegen range below emits it, and the
                // coverage tally counts it.  A library the test file `use`s is part
                // of that program however it got here, so a base must not move this
                // line — which is why a seeded file takes the boundary the BASE
                // recorded before its own libraries went in.
                p.data.definitions()
            };
            let parse_ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                p.parse(&abs_file, false);
            }));
            if let Err(payload) = parse_ok {
                let msg = panic_message(&*payload);
                if has_expect_error && ann.expect_errors.iter().any(|s| msg.contains(s.as_str())) {
                    println!("  ok    {display_name}  (expected parse error)");
                    total_files += 1;
                    dir_pass += 1;
                } else {
                    println!("  FAIL  {display_name}  (parse panic: {msg})");
                    dir_fail += 1;
                    total_files += 1;
                }
                continue;
            }

            // Scope analysis — wrap in catch_unwind so a panic here doesn't
            // kill the entire runner.
            //
            // loft#985 — this runs BEFORE the diagnostics are collected below, because the
            // post-scope lints report through the same channel and the collection is what
            // puts a diagnostic in front of the reader. It used to sit after the test
            // discovery further down, which is why nothing it produced could ever be seen.
            let scopes_ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                scopes::check(&mut p.data, &mut p.database);
            }));
            if let Err(payload) = scopes_ok {
                let msg = panic_message(&*payload);
                println!("  FAIL  {display_name}  (scope check panic: {msg})");
                dir_fail += 1;
                total_files += 1;
                continue;
            }
            // The same lint set the program path runs, ONCE per loaded file rather than
            // once per test: every test compiles its own bytecode from this one `Data`, so
            // a per-test call would report each finding N times. Until this ran, a
            // library's CI — which IS a test run — could not see a lost write, or a
            // `#superseded` steer pointing at nothing, which is a hard ERROR everywhere
            // else.
            loft::use_analysis::post_scope_lints(&p.data, &mut p.diagnostics, &abs_file);

            // Collect diagnostics.
            let mut file_result = FileResult {
                tests: Vec::new(),
                warnings: Vec::new(),
                advice: Vec::new(),
                errors: Vec::new(),
            };
            // The level, tolerating the `[code]` tag a coded diagnostic renders
            // (`Advice[superseded-call]: …`).  Matching `"Advice:"` alone sent every CODED
            // warning and advice into `errors`, so the file failed with "(parse errors)" —
            // giving a diagnostic its stable identity silently turned it into a build
            // break.  Anything unrecognised still counts as an error: a diagnostic this
            // cannot classify must not be quietly dropped.
            // The base's diagnostics first: they came from the library sources,
            // which a cold parse read before it reached the test file.
            for line in base_diagnostics.into_iter().chain(p.diagnostics.lines()) {
                match loft::diagnostics::compact_level(&line) {
                    Some(loft::diagnostics::Level::Warning) => {
                        file_result.warnings.push(line.clone());
                    }
                    Some(loft::diagnostics::Level::Advice) => {
                        file_result.advice.push(line.clone());
                    }
                    _ => file_result.errors.push(line.clone()),
                }
            }
            // @PLN86 / #631 — an admission violation fails the file, exactly as a
            // compile error does.  A rejected script cannot run at all, so a suite
            // that reported it green was reporting on something the host would refuse
            // to load.
            if p.has_sandboxed_defs() {
                sandbox_checked_files += 1;
                for e in p.sandbox_admission_errors() {
                    file_result.errors.push(format!("Sandbox admission: {e}"));
                }
            }

            // The two parser passes emit each diagnostic twice, so every warning and advice
            // reached the reader doubled.  A line carries its own position, so two identical
            // lines are the same finding said twice — never two findings (loft#948).
            {
                let mut seen = HashSet::new();
                file_result.warnings.retain(|l| seen.insert(l.clone()));
                let mut seen = HashSet::new();
                file_result.advice.retain(|l| seen.insert(l.clone()));
            }
            let has_fn_errors = !ann.expect_errors_fn.is_empty();
            let has_fn_warnings = !ann.expect_warnings_fn.is_empty();
            let all_warnings = file_result
                .warnings
                .iter()
                .chain(file_result.advice.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");

            // Per-function @EXPECT_ERROR: consume errors matching each function's
            // expected substrings.  Track which functions had their errors satisfied.
            let mut fn_error_pass: Vec<String> = Vec::new();
            let mut fn_error_fail: Vec<String> = Vec::new();
            // Substrings a function declared that nothing matched, as `fn: substring`.
            let mut fn_error_unmatched: Vec<String> = Vec::new();
            if has_fn_errors {
                for (fn_name, subs) in &ann.expect_errors_fn {
                    if file_result.errors.is_empty() {
                        fn_error_fail.push(fn_name.clone());
                        continue;
                    }
                    // `unexpected_errors` below walks the other direction — it rejects an
                    // error no annotation claims.  It cannot see an annotation that claims
                    // no error, which is why that filter is not the validation this needs.
                    let missing = unmatched_expect(subs, &file_result.errors);
                    if missing.is_empty() {
                        fn_error_pass.push(fn_name.clone());
                    } else {
                        for sub in missing {
                            fn_error_unmatched.push(format!("{fn_name}: {sub}"));
                        }
                    }
                }
            }

            // Per-function @EXPECT_WARNING: same logic.
            let mut fn_warning_pass: Vec<String> = Vec::new();
            let mut fn_warning_fail: Vec<String> = Vec::new();
            if has_fn_warnings {
                for (fn_name, subs) in &ann.expect_warnings_fn {
                    let matched = subs.iter().all(|s| all_warnings.contains(s.as_str()));
                    if matched {
                        fn_warning_pass.push(fn_name.clone());
                    } else {
                        fn_warning_fail.push(fn_name.clone());
                    }
                }
            }

            // Determine which errors are "unexpected" — not matched by any per-function
            // or file-level annotation.
            let unexpected_errors: Vec<&String> = if has_fn_errors || has_expect_error {
                file_result
                    .errors
                    .iter()
                    .filter(|e| {
                        // Consumed by a per-function annotation?
                        let fn_consumed = ann
                            .expect_errors_fn
                            .values()
                            .any(|subs| subs.iter().any(|s| e.contains(s.as_str())));
                        // Consumed by file-level annotation?
                        let file_consumed =
                            ann.expect_errors.iter().any(|s| e.contains(s.as_str()));
                        !fn_consumed && !file_consumed
                    })
                    .collect()
            } else {
                file_result.errors.iter().collect()
            };

            if !unexpected_errors.is_empty() {
                for e in &unexpected_errors {
                    println!("  {e}");
                }
                if !no_warnings {
                    // ADVICE prints here too, chained exactly as the success path below
                    // chains it.  Dropping it on the failure path silenced it in the one
                    // case where it is worth most: a diagnostic that explains a build break
                    // is only useful in the run that breaks (loft#948).
                    //
                    // `module-name-shadowed` is the case that was filed.  Two packages
                    // sharing a module file name resolve to one file, so the loser's
                    // functions are simply absent — reported as `Unknown function part_list`
                    // at a line inside a DEPENDENCY the consumer never edited.  The advice
                    // naming both files was produced all along and thrown away here, so the
                    // output that reached the author named neither the collision nor the fix
                    // (rename your own new file).  It printed only when the shadow happened
                    // to resolve and the build survived — the case a reader can already work
                    // out (loft#912's original diagnosis-by-elimination).
                    for w in file_result.warnings.iter().chain(file_result.advice.iter()) {
                        println!("  {w}");
                    }
                }
                println!("  FAIL  {display_name}  (parse errors)");
                dir_fail += 1;
                total_files += 1;
                continue;
            }
            // File-level @EXPECT_ERROR: if set but no errors matched, fail.
            if has_expect_error && file_result.errors.is_empty() {
                println!("  FAIL  {display_name}  (expected parse error but file parsed cleanly)");
                dir_fail += 1;
                total_files += 1;
                continue;
            }
            // …and EVERY substring must match one, the same bar `@EXPECT_WARNING` below
            // already holds itself to.  While one matching error satisfied all of them, a
            // file with three annotations and one live diagnostic passed, so an
            // expectation could be reworded out of existence and nothing would say so —
            // the `loft test` side of loft#929, where the same shape left 56 of the
            // harness's 167 annotations inert.
            let mut never_emitted: Vec<String> =
                unmatched_expect(&ann.expect_errors, &file_result.errors)
                    .into_iter()
                    .map(str::to_string)
                    .collect();
            never_emitted.extend(fn_error_unmatched);
            if !never_emitted.is_empty() {
                for e in &file_result.errors {
                    println!("  {e}");
                }
                println!(
                    "  FAIL  {display_name}  (expected error never emitted: {})",
                    never_emitted.join("; ")
                );
                dir_fail += 1;
                total_files += 1;
                continue;
            }
            // Per-function @EXPECT_ERROR that expected errors but none appeared.
            if !fn_error_fail.is_empty() && file_result.errors.is_empty() {
                println!(
                    "  FAIL  {display_name}  (expected errors for: {})",
                    fn_error_fail.join(", ")
                );
                dir_fail += 1;
                total_files += 1;
                continue;
            }

            // Check @EXPECT_WARNING (file-level): all substrings must match.
            let has_expect_warning = !ann.expect_warnings.is_empty();
            if has_expect_warning {
                let all_matched = ann
                    .expect_warnings
                    .iter()
                    .all(|sub| all_warnings.contains(sub.as_str()));
                if !all_matched {
                    let missing: Vec<&str> = ann
                        .expect_warnings
                        .iter()
                        .filter(|sub| !all_warnings.contains(sub.as_str()))
                        .map(String::as_str)
                        .collect();
                    for w in &file_result.warnings {
                        println!("  {w}");
                    }
                    println!(
                        "  FAIL  {display_name}  (expected warning not found: {})",
                        missing.join(", ")
                    );
                    dir_fail += 1;
                    total_files += 1;
                    continue;
                }
            }
            // Per-function @EXPECT_WARNING failures.
            if !fn_warning_fail.is_empty() {
                println!(
                    "  FAIL  {display_name}  (expected warnings not found for: {})",
                    fn_warning_fail.join(", ")
                );
                dir_fail += 1;
                total_files += 1;
                continue;
            }
            // A warning an `@EXPECT_WARNING` claims — file-level or per-function — is
            // intentional and stays quiet; every OTHER warning in the file is printed and
            // gated exactly as in a file that declares no expectation, the rule
            // `unexpected_errors` above already holds errors to.  Exempting the whole file
            // instead let an unrelated warning through a library's `--deny-warnings` CI
            // unseen: `regex`'s test pinned two `shadowed-by-method` warnings, and the
            // `both-receiver-deprecated` warning on its own source printed nothing and failed
            // nothing (C123).
            let claimed = |w: &String| {
                ann.expect_warnings.iter().any(|s| w.contains(s.as_str()))
                    || ann
                        .expect_warnings_fn
                        .values()
                        .any(|subs| subs.iter().any(|s| w.contains(s.as_str())))
            };
            let unexpected_warnings: Vec<&String> = file_result
                .warnings
                .iter()
                .filter(|w| !claimed(w))
                .collect();
            if !no_warnings {
                // Advice prints alongside warnings — it is reported, just never gated.
                for w in unexpected_warnings
                    .iter()
                    .copied()
                    .chain(file_result.advice.iter().filter(|a| !claimed(a)))
                {
                    println!("  {w}");
                }
            }
            // --deny-warnings (lib-CI gate): any warning no expectation claims fails the file.
            if deny_warnings && !unexpected_warnings.is_empty() {
                println!(
                    "  FAIL  {display_name}  (--deny-warnings: {} unexpected warning(s))",
                    unexpected_warnings.len()
                );
                dir_fail += 1;
                total_files += 1;
                continue;
            }

            // If the file has errors that were all expected (file-level or
            // per-function), report the passes and skip execution — the
            // compiler can't produce valid bytecode for a file with errors.
            if !file_result.errors.is_empty() {
                // All errors consumed → success.
                total_files += 1;
                for name in &fn_error_pass {
                    file_result.tests.push((name.clone(), true, None));
                    dir_pass += 1;
                }
                for name in &fn_warning_pass {
                    if !fn_error_pass.contains(name) {
                        file_result.tests.push((name.clone(), true, None));
                        dir_pass += 1;
                    }
                }
                let fn_names: Vec<&str> = file_result
                    .tests
                    .iter()
                    .map(|(n, _, _)| n.as_str())
                    .collect();
                let fn_list = fn_names.join(", ");
                let count = file_result.tests.len();
                println!(
                    "  ok    {display_name}  ({count} expected error{}: {fn_list})",
                    if count == 1 { "" } else { "s" }
                );
                continue;
            }
            // File-level @EXPECT_ERROR set but no errors at all → fail.
            if has_expect_error {
                println!("  FAIL  {display_name}  (expected parse error but file parsed cleanly)");
                dir_fail += 1;
                total_files += 1;
                continue;
            }
            // Per-function @EXPECT_ERROR but no errors at all → fail.
            if has_fn_errors && fn_error_fail.is_empty() && fn_error_pass.is_empty() {
                println!("  FAIL  {display_name}  (expected errors but file parsed cleanly)");
                dir_fail += 1;
                total_files += 1;
                continue;
            }

            // Coverage for THIS file's parse, indexed by `d_nr`; folded into the
            // run-wide map under stable identities once the file's tests have run.
            let mut file_entered: Vec<bool> = vec![false; p.data.definitions() as usize];

            // Find callable entry points: zero-parameter user functions, plus
            // single-vector-parameter functions when @ARGS provides argv.
            let has_user_args = !ann.user_args.is_empty();
            let mut test_fns: Vec<(u32, String)> = Vec::new();
            for d_nr in start_def..p.data.definitions() {
                let def = p.data.def(d_nr);
                if !matches!(def.def_type, DefType::Function) {
                    continue;
                }
                // Only user functions (n_<name>); skip generated lambdas.
                if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
                    continue;
                }
                // Skip standard library / operators.
                if crate::portable_path::is_stdlib_source(&def.position.file) {
                    continue;
                }
                // skip library functions loaded via `use`. Only run
                // functions defined in the test file itself.
                if def.position.file != abs_file {
                    continue;
                }
                // Zero parameters — always a test entry point.
                // Single vector<…> parameter — entry point when @ARGS provides argv.
                let attrs = &def.attributes;
                // Skip generator functions (return iterator<T>) — they're not tests.
                if matches!(def.returned, crate::data::Type::Iterator(_, _)) {
                    continue;
                }
                let callable = attrs.is_empty()
                    || (has_user_args
                        && attrs.len() == 1
                        && matches!(attrs[0].typedef, crate::data::Type::Vector(_, _)));
                if !callable {
                    continue;
                }
                let user_name = def.name.strip_prefix("n_").unwrap_or(&def.name);
                test_fns.push((d_nr, user_name.to_string()));
            }

            // loft#1010 — when the file names its tests, arity stops deciding.  Every
            // zero-parameter function used to be an entry point, so a `setup` helper ran
            // (and its `assert` could fail the suite), a `main` ran alongside the tests
            // with whatever it prints or writes, and the total counted both.  A parameter
            // was the only way to say "not a test", which is not something a reader would
            // guess.
            //
            // A file that declares at least one `test_*` has said which functions are
            // tests, so those are the whole set — @F89's rule, and what every runner one
            // language over does.  A file that declares NONE keeps arity: that is the
            // demonstration/script shape (1785 files in this corpus against 216 naming
            // `test_*`), and it is the only reason `--tests` can be pointed at a plain
            // program at all.
            //
            // Deliberately NOT the wrap harness's rule ("if `main` exists, only `main`
            // runs") — see TESTING.md § the two runners.  That one answers a different
            // question: the harness drives whole SCRIPTS, where `main` is the program.
            if test_fns.iter().any(|(_, n)| n.starts_with("test_")) {
                test_fns.retain(|(_, n)| n.starts_with("test_"));
            }

            // Apply function name filter (from "file.loft::name" syntax).
            if let Some(ref filter) = fn_filter {
                test_fns.retain(|(_, name)| filter.iter().any(|f| name == f));
            }

            if test_fns.is_empty() {
                // No callable functions found; skip this file silently.
                continue;
            }

            // Save the checked Data and raw Stores so each test function gets a
            // fresh State.  Stores::clone() preserves the type schema but resets
            // runtime allocations — State::new + compile::byte_code reinitialise
            // everything, giving each function a clean heap.
            let mut pending_native = p.pending_native_libs;
            // Merge extra native libs from loft.toml (the package's own native crate).
            for lib in extra_native_libs {
                if !pending_native.contains(lib) {
                    pending_native.push(lib.clone());
                }
            }
            let clean_data = p.data;
            // #255 / @PLN9: `source_dir` is now populated at parse time
            // (`Parser::parse`, the single home) — `p.database` already carries
            // the test file's directory, so no per-runner set is needed here.
            let clean_db = p.database;

            total_files += 1;

            if native_mode {
                // ── Native mode: generate Rust, compile, run ──────────────
                // Native codegen requires byte_code compilation first.
                let mut native_data = clean_data.clone();
                let mut native_state = State::new(clean_db.clone());
                compile::byte_code(&mut native_state, &mut native_data);
                // loft#907 — a `#native` symbol its library implements under
                // another Rust name has to be LINKED under that name, and only
                // the loaded cdylib's own registration says which.  The
                // interpreted branch below loads them for dispatch; the native
                // branch needs them for the same reason one step earlier, or a
                // remapped symbol compiles into a call to the wrong function.
                crate::extensions::load_all(&mut native_state, pending_native.clone());
                crate::extensions::resolve_native_impl_symbols(&mut native_data);
                let native_db = native_state.database;
                // Filter to functions that can run natively (skip @IGNORE,
                // @EXPECT_ERROR, and @EXPECT_FAIL — native can't catch panics).
                let mut native_fns: Vec<(u32, String)> = Vec::new();
                for (d_nr, fn_name) in &test_fns {
                    if ann.ignore_fn.contains(fn_name.as_str()) {
                        file_result.tests.push((
                            fn_name.clone(),
                            true,
                            Some("ignored".to_string()),
                        ));
                        continue;
                    }
                    if ann.expect_errors_fn.contains_key(fn_name.as_str()) {
                        continue;
                    }
                    let should_fail = ann.expect_fail_fn.contains_key(fn_name.as_str())
                        || !ann.expect_fail_file.is_empty();
                    if should_fail {
                        file_result.tests.push((
                            fn_name.clone(),
                            true,
                            Some("skip-native".to_string()),
                        ));
                        continue;
                    }
                    native_fns.push((*d_nr, fn_name.clone()));
                }
                if native_fns.is_empty() {
                    // Nothing to run natively — record as pass with note.
                    if file_result.tests.is_empty() {
                        file_result.tests.push((
                            "(no native tests)".to_string(),
                            true,
                            Some("skip-native".to_string()),
                        ));
                    }
                } else {
                    // Generate Rust source.
                    let end_def = native_data.definitions();
                    let main_nr = native_data.def_nr("n_main");
                    // loft#1010 — the file's own `main` is the crate's entry point only when
                    // this run means to CALL it.  It used to be enough that `n_main` existed:
                    // the generated binary then ran `main` and nothing else, exited 0, and the
                    // harness reported every `test_*` as passed WITHOUT running one.  A test
                    // asserting `1 == 2` beside a `main` was green on `--native` and red on the
                    // interpreter — a false green, which is the one failure mode a test runner
                    // may not have.
                    //
                    // `native_fns` is the authority on what this run calls (loft#1010 narrows it
                    // to `test_*` whenever the file names any).  When `main` is not in it, the
                    // harness supplies the crate's `main` below and the generator's own bootstrap
                    // is suppressed — the path #621 already built for a package whose `src/`
                    // entry also carries an `n_main`.
                    let has_main = main_nr < end_def
                        && native_data.def(main_nr).name == "n_main"
                        && native_fns.iter().any(|(_, n)| n == "main");
                    let entry_defs: Vec<u32> = if has_main {
                        vec![main_nr]
                    } else {
                        native_fns.iter().map(|(d, _)| *d).collect()
                    };
                    let gen_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut buf: Vec<u8> = Vec::new();
                        let mut out = generation::Output::new(&native_data, &native_db);
                        // Host-native backend: C-ABI cdylib link (NATIVE.md § Resolution).
                        out.native_cabi = native_utils::native_cabi_enabled();
                        // #621 — when this harness supplies the crate's `main`
                        // below, suppress the generator's own bootstrap.
                        // `output_native_reachable` emits one whenever ANY
                        // definition is named `n_main`, and that scan is not
                        // source-scoped: a tested package whose `src/` entry is
                        // also a runnable CLI contributes an `n_main`, so both
                        // mains landed in one crate (rustc E0428) and every test
                        // in an 11-of-48-packages shape was untestable natively.
                        // `has_main` (source-scoped) is the right authority for
                        // whose entry point this is — the TEST FILE's, if any.
                        if has_main {
                            out.output_native_reachable(&mut buf, start_def, end_def, &entry_defs)
                                .expect("native codegen write");
                        } else {
                            out.output_native_no_bootstrap(
                                &mut buf,
                                start_def,
                                end_def,
                                &entry_defs,
                            )
                            .expect("native codegen write");
                        }
                        // output_native_reachable emits fn main() when n_main
                        // exists.  For test-only files (no n_main) we generate
                        // a main() that calls each test function.
                        if !has_main {
                            use std::io::Write;
                            // P199 — wrap Stores in UnsafeCell so the new
                            // ABI's `&UnsafeCell<Stores>` parameter type is
                            // satisfied by the entry call.
                            writeln!(buf, "\nfn main() {{").unwrap();
                            writeln!(
                                buf,
                                "    let cell = std::cell::UnsafeCell::new(Stores::new());"
                            )
                            .unwrap();
                            // #255: anchor file I/O at the TEST file's directory
                            // (handed down by the runner via LOFT_SOURCE_DIR) so
                            // `file()` / `source_dir()` resolve against the test's
                            // assets — matching the interpreter and the standalone
                            // `--native` main.  A fresh `Stores::new()` otherwise
                            // has an empty `source_dir` anchor → resolve_path falls
                            // back to passthrough (the process cwd / scratch dir),
                            // so a relative `file("asset")` missed under --native
                            // while passing under the interpreter.
                            //
                            // The anchor MODE is the program's own, not a constant:
                            // a `#cwd` file asks for cwd-relative paths, and baking
                            // `true` here compiled it as though it had never written
                            // the directive.  `source_dir` is set either way — it is
                            // what the `source_dir()` built-in answers, and
                            // `resolve_path` ignores it when the mode is cwd.
                            writeln!(
                                buf,
                                "    {{ let s: &mut Stores = unsafe {{ &mut *cell.get() }}; s.source_dir = Stores::source_dir_native(); s.program_relative = {}; }}",
                                clean_db.program_relative
                            )
                            .unwrap();
                            writeln!(buf, "    init(&cell);").unwrap();
                            // The harness names ONE test per run (its first argument), so a
                            // failure is attributed to the test that failed and every test
                            // starts from fresh stores, as it does on the interpreter.  With
                            // no argument the binary runs them all.
                            writeln!(buf, "    let only = std::env::args().nth(1);").unwrap();
                            for (_, name) in &native_fns {
                                writeln!(
                                    buf,
                                    "    if only.as_deref().is_none_or(|n| n == \"{name}\") {{ n_{name}(&cell); }}"
                                )
                                .unwrap();
                            }
                            writeln!(buf, "}}").unwrap();
                        }
                        buf
                    }));
                    let buf = match gen_result {
                        Ok(b) => b,
                        Err(payload) => {
                            let msg = panic_message(&*payload);
                            for (_, fn_name) in &native_fns {
                                file_result.tests.push((
                                    fn_name.clone(),
                                    false,
                                    Some(format!("native codegen panic: {msg}")),
                                ));
                                dir_fail += 1;
                            }
                            // Skip compile+run phases.
                            Vec::new()
                        }
                    };
                    if !buf.is_empty() {
                        let stem = std::path::Path::new(&abs_file)
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .replace('-', "_");
                        // The scratch directory is shared by every process on the box —
                        // every test binary of a gate, and every checkout's gate — and one
                        // file is often compiled several times at once with DIFFERENT
                        // emissions (`keyed_fast_paths` runs `158-keyed-fast-paths` under
                        // each `LOFT_KEYED_VERIFY` setting).  So nothing is written at a
                        // shared path while another process may read it: the source and the
                        // binary are built in this process's own directory, and the binary
                        // is PUBLISHED under its cache key by an atomic rename.  A shared
                        // per-stem name let one process rewrite the source another was
                        // compiling (`unexpected closing delimiter`), truncate the binary
                        // another was linking (`ld terminated with signal 7`), or hand it a
                        // program built against another checkout's rlib (`undefined hidden
                        // symbol`) — loft#1626.  Equal keys mean equal programs, so a
                        // published binary is never replaced by a different one.
                        let scratch = crate::platform::scratch_dir();
                        let lib_dir = native_utils::loft_lib_dir();
                        let key = native_utils::native_cache_key(
                            &buf,
                            lib_dir.as_deref(),
                            Some(&native_data),
                        );
                        let binary =
                            scratch.join(format!("loft_test_native_{stem}_{key:016x}_bin"));
                        let work = crate::platform::build_scratch_dir("test_native");
                        let tmp_rs = work.join(format!("loft_test_native_{stem}.rs"));
                        let tmp_bin = work.join(format!("loft_test_native_{stem}_bin"));
                        let cached = binary.exists();
                        if !cached {
                            let _ = std::fs::write(&tmp_rs, &buf);
                        }

                        // Layer 2: never start a compile that could overflow a
                        // RAM-backed tmpfs (reclaims loft's own stale artefacts
                        // first).  Warn + proceed; rustc surfaces a real ENOSPC
                        // if it genuinely can't write.
                        if !cached && !crate::platform::native_compile_space_ok(&scratch) {
                            eprintln!(
                                "loft: warning — low space in {} (set LOFT_TMPFS_MIN_FREE_MB to tune)",
                                scratch.display()
                            );
                        }
                        let compile_ok = if cached {
                            crate::platform::timing_record("fixture", &stem, true, None);
                            true
                        } else {
                            // Compile with rustc.
                            let mut cmd = std::process::Command::new("rustc");
                            // Keep rustc's own intermediates in the loft
                            // scratch dir too, so the whole native compile
                            // stays off a small `/tmp` tmpfs.
                            cmd.env("TMPDIR", &scratch)
                                .arg("--edition=2024")
                                .arg("-C")
                                .arg("debuginfo=0")
                                .arg("-C")
                                .arg("opt-level=0")
                                .arg("-o")
                                .arg(&tmp_bin)
                                .arg(&tmp_rs);
                            // Layer 1: strip the linked binary (~36MB → ~1MB;
                            // the bulk is debug info from libloft.rlib + std,
                            // useless to a run-and-check test).  Opt out with
                            // LOFT_NATIVE_KEEP_SYMBOLS=1 when debugging a crash.
                            if crate::platform::native_strip_symbols() {
                                cmd.arg("-C").arg("strip=symbols");
                            }
                            // @P389: each native package's rlib carries its own
                            // copy of `loft_register_v1` (synthesized by the
                            // `loft_ffi::loft_register!` macro for the cdylib's
                            // dlopen registration path).  When a test file pulls
                            // TWO OR MORE native packages into the SAME binary
                            // (e.g. `use server; use web;` driving an HTTP
                            // round-trip), `ld` errors with `duplicate symbol:
                            // loft_register_v1`.  The binary never calls that
                            // symbol — it inlines `loft_<crate>::n_…` directly —
                            // so the duplicates are functionally harmless; tell
                            // the linker to keep the first definition and skip
                            // the rest.  This mirrors the identical mitigation on
                            // the standalone `--native` path in `main.rs`; this
                            // path (the `--tests --native` / `loft test` runner)
                            // built its own rustc command and never got it, so
                            // single-package smokes passed but any genuine
                            // two-native-package test failed at the link step.
                            // macOS ld64 rejects `--allow-multiple-definition`
                            // and MSVC `link.exe` ignores it with a `LNK4044`
                            // per occurrence, so skip it on both (matching
                            // main.rs).
                            #[cfg(not(any(target_os = "macos", windows)))]
                            if !native_data.native_packages.is_empty() {
                                cmd.arg("-Clink-arg=-Wl,--allow-multiple-definition");
                            }
                            if let Some(ref ld) = lib_dir {
                                cmd.args(loft::native_lib::loft_extern_args(
                                    &ld.join("libloft.rlib"),
                                ));
                                cmd.arg("-L").arg(native_utils::deps_dir_of(ld));
                                // Propagate `-L native=` for every build-script
                                // `OUT_DIR` that bundles a native lib — the G2
                                // mitigation main.rs already has on the standalone
                                // path.  windows-targets ships `windows.0.XX.0.lib`
                                // inside its OUT_DIR; without these the Windows link
                                // fails `LNK1181: cannot open input file
                                // 'windows.0.XX.0.lib'`.  Native packages that pull
                                // windows-targets via their OWN deps masked this (the
                                // rlib branch of `add_native_extern_flags` harvests
                                // the PACKAGE's OUT_DIRs), but a dependency-free
                                // `[native] crate` package brings none — so the test
                                // path needs loft's own OUT_DIRs too.
                                for out_dir in native_utils::build_script_native_lib_dirs(ld) {
                                    cmd.arg("-L").arg(format!("native={}", out_dir.display()));
                                }
                                // The C-ABI native consumer names `loft_ffi` types
                                // (LoftStore/LoftRef/LoftStr) in its `extern "C"`
                                // decls, so loft's own `loft_ffi` rlib must be on
                                // the command (mirrors the standalone native
                                // compile in main.rs).  Pick the copy `libloft` was
                                // built against, not the first in dir order — two
                                // copies in `deps/` else collide on StableCrateId
                                // (see `native_lib::loft_ffi_for_libloft`).
                                if let Some(ffi) = loft::native_lib::loft_ffi_for_libloft(
                                    &ld.join("libloft.rlib"),
                                    &native_utils::deps_dir_of(ld),
                                ) {
                                    cmd.arg("--extern")
                                        .arg(format!("loft_ffi={}", ffi.display()));
                                }
                            }
                            // LibCI: link each package's `#native` crate so tests
                            // for native-backed libraries (graphics, crypto, …)
                            // compile under --native — mirrors the standalone +
                            // WASM native compiles, which already call this.
                            let loft_deps = lib_dir.as_ref().map(|d| native_utils::deps_dir_of(d));
                            native_utils::add_native_extern_flags(
                                &mut cmd,
                                &native_data,
                                None,
                                loft_deps.as_deref(),
                            );
                            let rustc_start = std::time::Instant::now();
                            let compile_result = cmd.output();
                            crate::platform::timing_record(
                                "fixture",
                                &stem,
                                false,
                                Some(rustc_start.elapsed().as_secs_f64()),
                            );
                            let ok = compile_result
                                .as_ref()
                                .map(|o| o.status.success())
                                .unwrap_or(false);
                            let ok = ok
                                && (std::fs::rename(&tmp_bin, &binary).is_ok() || binary.exists());
                            if !ok {
                                let stderr_msg = compile_result.as_ref().ok().map_or_else(
                                    || "rustc not found".to_string(),
                                    |o| {
                                        let s = String::from_utf8_lossy(&o.stderr);
                                        let err = s
                                            .lines()
                                            .find(|l| l.starts_with("error"))
                                            .unwrap_or("(unknown)");
                                        // Append linker-detail lines so a link failure
                                        // NAMES its cause instead of just "exit code:
                                        // 1181" / "exit status: 1" — the first `error:`
                                        // line alone hid it.
                                        //
                                        // The first three markers are the WINDOWS shapes
                                        // and were the whole list until 2026-09-23, when
                                        // `keyed_fast_paths` went red inside three
                                        // different agents' gates on this box and green
                                        // standalone in all three.  Every one of us called
                                        // it environmental without evidence, because a
                                        // UNIX `cc` failure names its cause in
                                        // `collect2:` / `ld:` lines that match none of the
                                        // three — so the report said only that linking
                                        // failed.  An OOM kill during parallel linking and
                                        // a full disk are the two this box actually
                                        // produces, and both are one line away from being
                                        // self-evident.
                                        let detail: Vec<&str> = s
                                            .lines()
                                            .filter(|l| {
                                                l.contains("LNK")
                                                    || l.contains("cannot open")
                                                    || l.contains("undefined")
                                                    || l.contains("collect2")
                                                    || l.contains("ld:")
                                                    || l.contains("ld returned")
                                                    || l.contains("No space left")
                                                    || l.contains("Killed")
                                                    || l.contains("terminated with signal")
                                            })
                                            .take(4)
                                            .collect();
                                        if detail.is_empty() {
                                            err.to_string()
                                        } else {
                                            format!("{err} | {}", detail.join(" | "))
                                        }
                                    },
                                );
                                let _ = std::fs::remove_file(&tmp_bin);
                                for (_, fn_name) in &native_fns {
                                    file_result.tests.push((
                                        fn_name.clone(),
                                        false,
                                        Some(format!("native compile: {stderr_msg}")),
                                    ));
                                    dir_fail += 1;
                                }
                            }
                            ok
                        };

                        if compile_ok {
                            // Run the compiled binary.  Hand down the source
                            // anchor — the TEST file's OWN directory — so
                            // `source_dir()` and program-relative file I/O
                            // resolve against the test's assets, not the scratch
                            // dir where the generated binary lives.  Without this
                            // `source_dir_native()` falls back to the executable's
                            // dir (the loft scratch/tmp), so a relative
                            // `file("asset")` or `source_dir()` misses under
                            // `--native` while passing under the interpreter
                            // (which anchors at the test file's dir).  Mirrors the
                            // standalone `--native` run path in main.rs; an
                            // explicit user `LOFT_SOURCE_DIR` wins.
                            // @PLN26 phase 4 — stage native-package DLLs beside the
                            // test binary on Windows (no RPATH there); mirrors the
                            // standalone run path in main.rs.  No-op off Windows /
                            // on the rlib path.
                            if let Some(dir) = binary.parent() {
                                native_utils::stage_native_dlls(dir, &native_data);
                            }
                            let run_one = |only: Option<&str>| -> Result<(), String> {
                                let mut run_cmd = std::process::Command::new(&binary);
                                if let Some(name) = only {
                                    run_cmd.arg(name);
                                }
                                if std::env::var("LOFT_SOURCE_DIR").is_err()
                                    && let Some(dir) = std::path::Path::new(&abs_file).parent()
                                {
                                    run_cmd.env("LOFT_SOURCE_DIR", dir);
                                }
                                // Run the native test binary with cwd = source_dir so its
                                // raw `std::fs` (e.g. imaging's load_png/save_png) anchors
                                // where its loft `file()` does.  Gated on the program's own
                                // mode, exactly as `enter_source_dir` gates the in-process
                                // interpreter run: a `#cwd` program anchors loft I/O at the
                                // cwd, so moving the child would put its raw `std::fs`
                                // somewhere its `file()` is not.
                                if clean_db.program_relative
                                    && let Some(dir) = std::path::Path::new(&abs_file).parent()
                                {
                                    run_cmd.current_dir(dir);
                                }
                                // The child's output is passed on as it was; its stderr is
                                // also read for the first `error:` line, which names why
                                // the test failed.
                                run_cmd.stdout(std::process::Stdio::inherit());
                                let out = run_cmd
                                    .output()
                                    .map_err(|e| format!("native run failed: {e}"))?;
                                let stderr = String::from_utf8_lossy(&out.stderr);
                                eprint!("{stderr}");
                                if out.status.success() {
                                    return Ok(());
                                }
                                Err(native_failure(out.status, &stderr))
                            };
                            // A file's own `main` is one run; otherwise one run per test.
                            let names: Vec<Option<&str>> = if has_main {
                                vec![None]
                            } else {
                                native_fns.iter().map(|(_, n)| Some(n.as_str())).collect()
                            };
                            for only in names {
                                let outcome = run_one(only);
                                // A whole-file run (`main`) stands for every entry in it.
                                let covered: Vec<&String> = match only {
                                    Some(name) => native_fns
                                        .iter()
                                        .map(|(_, n)| n)
                                        .filter(|n| n.as_str() == name)
                                        .collect(),
                                    None => native_fns.iter().map(|(_, n)| n).collect(),
                                };
                                for fn_name in covered {
                                    match &outcome {
                                        Ok(()) => {
                                            file_result.tests.push((fn_name.clone(), true, None));
                                            dir_pass += 1;
                                        }
                                        Err(msg) => {
                                            file_result.tests.push((
                                                fn_name.clone(),
                                                false,
                                                Some(msg.clone()),
                                            ));
                                            dir_fail += 1;
                                        }
                                    }
                                }
                            }
                        }
                        // The published binary stays as the cache; this process's own
                        // build directory goes.  `LOFT_KEEP_NATIVE_RS=1` keeps it, source
                        // included, for inspection.
                        if std::env::var_os("LOFT_KEEP_NATIVE_RS").is_none() {
                            let _ = std::fs::remove_dir_all(&work);
                        }
                    }
                }
            } else {
                // ── Interpreter mode ──────────────────────────────────────────
                // Anchor native file I/O at the program's source_dir for the
                // duration of this file's tests; the guard restores the cwd
                // afterwards so the next file's parse/compile is unaffected.
                let _cwd = enter_source_dir(&clean_db.source_dir, clean_db.program_relative);
                for (_, fn_name) in &test_fns {
                    // Per-function @IGNORE: skip without running.
                    if ann.ignore_fn.contains(fn_name.as_str()) {
                        file_result.tests.push((
                            fn_name.clone(),
                            true,
                            Some("ignored".to_string()),
                        ));
                        continue;
                    }
                    // Per-function @EXPECT_ERROR: already counted, don't execute.
                    if ann.expect_errors_fn.contains_key(fn_name.as_str()) {
                        continue;
                    }
                    let fn_name_owned = fn_name.clone();
                    let user_args = ann.user_args.clone();
                    let production = ann.production;
                    let log_conf = ann.log_conf.clone();

                    // Build a fresh State + bytecode for every function so tests
                    // within a file cannot leak heap/store state into each other.
                    let loft_log_active = std::env::var("LOFT_LOG").is_ok();
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut data_copy = clean_data.clone();
                        let mut state = State::new(clean_db.clone());
                        compile::byte_code(&mut state, &mut data_copy);
                        // Load native extensions for packages with #native functions.
                        crate::extensions::load_all(&mut state, pending_native.clone());
                        // PKG.5: wire auto-marshalled native functions.
                        crate::extensions::wire_native_fns(&mut state, &data_copy);

                        // Set up logger if @ARGS requested --production or --log-conf.
                        if production || log_conf.is_some() {
                            let lg = if let Some(ref conf) = log_conf {
                                let cp = std::path::PathBuf::from(conf);
                                crate::logger::Logger::from_config_file(&cp, &abs_file)
                            } else {
                                crate::logger::Logger::production()
                            };
                            let mut lg = lg;
                            if production {
                                lg.config.production = true;
                            }
                            state.database.logger =
                                Some(std::sync::Arc::new(std::sync::Mutex::new(lg)));
                        }

                        // Arm coverage for this run.  Sized to the definition table so
                        // the hook is a bounds-checked index, never a resize.
                        state.entered_fns = Some(vec![false; data_copy.definitions() as usize]);
                        // loft#860 — and the profiler, if the environment asked for it.
                        // A no-op otherwise: `Profiler::from_env` returns `None`, so an
                        // ordinary test run pays a single `var_os` per test.
                        state.arm_profiler();
                        if loft_log_active {
                            // When LOFT_LOG is set, emit IR+bytecode+trace to stderr
                            // (same format as cargo-test dump files, but to stderr so
                            // it is visible immediately for ad-hoc --tests invocations).
                            let config = LogConfig::from_env();
                            let mut log = std::io::stderr();
                            writeln!(log, "=== {fn_name_owned} ===").ok();
                            compile::show_code(&mut log, &mut state, &mut data_copy, &config).ok();
                            if let Err(e) =
                                state.execute_log(&mut log, &fn_name_owned, &config, &data_copy)
                            {
                                panic!("{e}");
                            }
                        } else {
                            state.execute_argv(&fn_name_owned, &data_copy, &user_args);
                        }
                        // A frame-driven program (`yield_frame`, `gl_swap_buffers`) hands
                        // control back after each frame and is RESUMED until it finishes —
                        // the CLI's loop.  Without it the runner scored the first frame as
                        // the whole test and abandoned `main` mid-loop: its scope-exit frees
                        // never ran, and the release valgrind sweep (which runs every script
                        // under `--tests`) reported the frame's formatted texts as definitely
                        // lost (loft#1357, `85-yield-resume`).
                        while state.database.frame_yield {
                            state.resume();
                        }
                        // loft#860 — resolve this test's samples against ITS `Data` and
                        // merge them.  Before the fault check below, because a test
                        // that FAULTED still burnt the time it burnt; only a Rust panic
                        // (caught outside) loses its samples, and that is an aborted run
                        // rather than a measured one.
                        state.fold_profile(&data_copy, &mut profile.borrow_mut());
                        // Fold this test's entries into the file's tally.  A function
                        // reached by any one test in the file counts as reached.
                        if let Some(seen) = state.entered_fns.take() {
                            for (i, hit) in seen.iter().enumerate() {
                                if *hit && i < file_entered.len() {
                                    file_entered[i] = true;
                                }
                            }
                        }
                        // @P367: assert(false) / panic / divide-by-zero / OOB /
                        // null-deref set a *typed* runtime fault and halt WITHOUT
                        // a Rust panic (the C66 path), so catch_unwind sees Ok and
                        // the test would otherwise be scored PASSED.  Surface the
                        // fault so the runner can FAIL the test — and so an
                        // intentional fault still satisfies @EXPECT_FAIL.
                        // `had_fatal` is set in both dev and production modes;
                        // `runtime_error` carries the message (dev mode).
                        let fault = state.database.had_fatal;
                        let fault_msg = state
                            .database
                            .runtime_error
                            .as_ref()
                            .map(|e| e.message.clone())
                            .unwrap_or_else(|| "runtime error".to_string());
                        (fault, fault_msg)
                    }));

                    // Evaluate pass/fail, respecting @EXPECT_FAIL annotations.
                    // A function "should fail" when it has a per-function
                    // @EXPECT_FAIL, or when a file-level @EXPECT_FAIL applies
                    // (and no per-function annotation overrides it).
                    let should_fail = ann.expect_fail_fn.contains_key(fn_name.as_str())
                        || (!ann.expect_fail_file.is_empty()
                            && !ann.expect_fail_fn.contains_key(fn_name.as_str()));
                    let (passed, fail_msg) = match result {
                        Ok((fault, fault_msg)) => {
                            if fault {
                                // @P367: a typed runtime fault fired (no Rust panic).
                                if should_fail && matches_expect_fail(&ann, fn_name, &fault_msg) {
                                    (true, None) // expected fault — pass
                                } else if should_fail {
                                    (
                                        false,
                                        Some(format!(
                                            "failed, but the error did not match @EXPECT_FAIL: {fault_msg}"
                                        )),
                                    )
                                } else {
                                    (false, Some(fault_msg)) // fault now FAILS the test
                                }
                            } else if should_fail {
                                (
                                    false,
                                    Some(
                                        "expected failure but function returned cleanly"
                                            .to_string(),
                                    ),
                                )
                            } else {
                                (true, None)
                            }
                        }
                        Err(payload) => {
                            let msg = panic_message(&*payload);
                            if should_fail && matches_expect_fail(&ann, fn_name, &msg) {
                                (true, None) // expected failure — pass
                            } else {
                                (false, Some(msg))
                            }
                        }
                    };

                    file_result.tests.push((fn_name.clone(), passed, fail_msg));
                    if passed {
                        dir_pass += 1;
                    } else {
                        dir_fail += 1;
                    }
                }
            } // end interpreter mode

            // Per-file summary line.
            let ignored_count = file_result
                .tests
                .iter()
                .filter(|(_, _, m)| m.as_deref() == Some("ignored"))
                .count();
            total_skipped += u32::try_from(
                file_result
                    .tests
                    .iter()
                    .filter(|(_, _, m)| m.as_deref() == Some("skip-native"))
                    .count(),
            )
            .unwrap_or(0);
            let pass_count = file_result
                .tests
                .iter()
                .filter(|(_, p, m)| *p && m.as_deref() != Some("ignored"))
                .count();
            let fail_count = file_result.tests.len() - pass_count - ignored_count;
            let fn_names: Vec<&str> = file_result
                .tests
                .iter()
                .filter(|(_, _, m)| m.as_deref() != Some("ignored"))
                .map(|(n, _, _)| n.as_str())
                .collect();
            let fn_list = fn_names.join(", ");
            if fail_count == 0 {
                let ignore_note = if ignored_count > 0 {
                    format!(", {ignored_count} ignored")
                } else {
                    String::new()
                };
                println!(
                    "  ok    {display_name}  ({pass_count} fn{}{ignore_note}: {fn_list})",
                    if pass_count == 1 { "" } else { "s" }
                );
            } else {
                for (name, passed, msg) in &file_result.tests {
                    if !passed {
                        if let Some(m) = msg {
                            println!("  FAIL  {display_name}::{name}  —  {m}");
                        } else {
                            println!("  FAIL  {display_name}::{name}");
                        }
                    }
                }
                println!("  FAIL  {display_name}  ({fail_count} failed, {pass_count} passed)");
            }

            let pkg_root = package_root_for(&abs_file);
            // Fold this file's tally into the run-wide map.  What counts is the code
            // UNDER TEST: definitions from this parse (so, past the stdlib boundary at
            // `start_def`), excluding the test file's own driver functions and anything
            // pulled in from a dependency — a package is not answerable for its deps'
            // coverage.
            for d_nr in start_def..clean_data.definitions() {
                let def = clean_data.def(d_nr);
                if !matches!(def.def_type, DefType::Function) {
                    continue;
                }
                // Decode via the canonical mapper: a library's API is mostly METHODS,
                // stored as `t_<LEN><Type>_<method>`, so a bare `n_` prefix check would
                // silently count only the free functions — `arguments` reported 4 of
                // its 21 that way.  Lambdas are generated, not written, so they are not
                // the author's to cover.
                if def.name.starts_with("n___lambda_") {
                    continue;
                }
                let Some((_kind, shown)) = loft::api_surface::classify(&clean_data, d_nr) else {
                    continue;
                };
                // A `#native` declaration has no loft body to enter — it dispatches
                // straight to Rust — so it can never be recorded and would otherwise
                // report as permanently uncovered.  A native-backed package would then
                // read 100% uncovered however well it is tested, which is exactly the
                // kind of lying number that gets a coverage report ignored.
                if !def.native.is_empty() {
                    continue;
                }
                let Some(src) = coverage_path(&def.position.file, &abs_file, pkg_root.as_deref())
                else {
                    continue;
                };
                let key = (src, def.position.line, shown);
                let hit = file_entered.get(d_nr as usize).copied().unwrap_or(false);
                let slot = coverage.entry(key).or_insert(false);
                *slot = *slot || hit;
            }
        }

        // Per-directory summary.
        if dir_pass + dir_fail > 0 {
            dir_summaries.push((dir_path.clone(), dir_pass, dir_fail));
            total_pass += dir_pass;
            total_fail += dir_fail;
        }
    }

    // Restore the default panic hook.
    std::panic::set_hook(prev_hook);

    // Final summary.
    println!();
    if dir_summaries.len() > 1 {
        for (dir_path, pass, fail) in &dir_summaries {
            if *fail == 0 {
                println!("  {dir_path}: {pass} passed");
            } else {
                println!("  {dir_path}: {fail} failed, {pass} passed");
            }
        }
        println!();
    }

    // The result states WHICH backend produced it.  `loft test` and
    // `loft test --native` each exercise exactly one, so a bare "ok" used to be
    // identical whether the other backend was clean or had never been compiled
    // once — silence read as coverage.  A consumer discovered a quarter of their
    // packages had never been native-compiled while `loft test` stayed green
    // throughout, and could only find out by running the native sweep by hand.
    // The scope is not optional and not behind a flag: it rides on the default
    // path, because that is the path that was lying.
    let (ran, missing, missing_cmd) = if native_mode {
        ("native", "the interpreter", "loft test")
    } else {
        ("the interpreter", "native", "loft test --native")
    };
    // Tests that were counted but never executed on `ran` (see `total_skipped`).
    let skipped = if total_skipped > 0 {
        format!(", {total_skipped} skipped")
    } else {
        String::new()
    };
    // Same rule for admission: state whether it covered anything.  A suite that ran
    // with no policy in sight is silent about admission, and a consumer read that
    // silence as coverage — they injected a deliberate capability violation and the
    // suite stayed green.
    let admission = if sandbox_checked_files > 0 {
        format!(
            "; admission checked on {sandbox_checked_files} file{}",
            if sandbox_checked_files == 1 { "" } else { "s" }
        )
    } else if sandbox_policy_seen {
        "; a [sandbox] policy is present but designated nothing here — admission \
         covered NO code (check the selectors)"
            .to_string()
    } else {
        "; no [sandbox] policy — admission not exercised".to_string()
    };
    let scope =
        format!("[ran on {ran} only{skipped} — {missing} not exercised: {missing_cmd}{admission}]");

    // Function coverage: name the functions the suite never entered.  A LIST, not a
    // percentage — a percentage becomes a target, and a coverage target is what makes
    // people write tests that reach a line instead of tests that check a behaviour.
    // Every entry here is an individual, checkable fact: this code did not run.
    //
    // Native mode compiles each function to Rust, so there is no `fn_call` to observe
    // and nothing to report; the interpreter leg carries this.
    let unreached: Vec<&(String, u32, String)> = coverage
        .iter()
        .filter(|(_, hit)| !**hit)
        .map(|(k, _)| k)
        .collect();
    // An empty map means there was nothing to measure (a loose script, or a package
    // whose code is all `#native`) — say nothing rather than imply a clean result.
    // A full map with nothing unreached is a real result and gets said out loud, so
    // "no coverage line" can never be mistaken for "everything is covered".
    if !native_mode && !coverage.is_empty() && unreached.is_empty() {
        println!(
            "coverage: all {} functions were entered by these tests",
            coverage.len()
        );
    }
    if !native_mode && !coverage.is_empty() && !unreached.is_empty() {
        println!(
            "coverage: {} of {} functions were never entered by these tests",
            unreached.len(),
            coverage.len()
        );
        // Show enough to act on without burying the test result; the rest on request.
        // `LOFT_COVERAGE=list` prints every entry — for piping into a worklist rather
        // than reading at the end of a run.
        let full = std::env::var("LOFT_COVERAGE").is_ok_and(|v| v == "list");
        const SHOWN: usize = 10;
        let shown = if full { unreached.len() } else { SHOWN };
        for (file, line, name) in unreached.iter().take(shown) {
            println!("  {file}:{line}  {name}");
        }
        if unreached.len() > shown {
            println!(
                "  … and {} more (LOFT_COVERAGE=list for the full list)",
                unreached.len() - shown
            );
        }
    }

    // loft#860 — one merged profile for the whole suite, after the per-file output so
    // it is the last thing on the screen.  `report` is silent when nothing was armed.
    profile.borrow().report();
    // ...and say so for the one profiling instrument a suite CANNOT answer, rather
    // than accepting the variable and printing nothing.  `LOFT_ALLOC_SITES` ranks a
    // process-global peak by the `pc` that allocated, and a suite's peak may have been
    // reached in any of its runs — every one of which compiled its own bytecode.  So
    // there is no `Data` those positions can be resolved against, and resolving them
    // against whichever run happened to finish last would name real lines in the wrong
    // file: the table would look exactly like an answer.
    if loft::store_budget::sites_armed() {
        eprintln!(
            "loft: LOFT_ALLOC_SITES is not available under a test run.\n  \
             The ledger ranks a process-wide peak by bytecode position, and each test \
             compiles its own\n  bytecode, so those positions cannot be resolved to \
             lines. LOFT_PROFILE and LOFT_ALLOC_PATHS\n  do work here; for the heap \
             ledger, run the code as a program (loft --interpret prog.loft)."
        );
    }

    let total = total_pass + total_fail;
    // A `::selector` that matched NO test function must not report success. The
    // per-file filter leaves such a file with nothing to run and skips it silently,
    // so a mistyped or renamed test name came out as `ok. 0 passed; 0 files` — a
    // green that means nothing, and the shape a CI job reads as "the tests I asked
    // for passed". Only an explicit selector is checked: a directory with no tests
    // in it is a different, legitimate zero.
    if total_files == 0
        && total_fail == 0
        && let Some(ref filter) = fn_filter
    {
        println!(
            "loft: no test function named {} in '{path_part}'",
            filter
                .iter()
                .map(|f| format!("'{f}'"))
                .collect::<Vec<_>>()
                .join(" or ")
        );
        return 1;
    }
    // The text-buffer ledger (`LOFT_TEXT_TIMELINE`) reports here as it does at a program's
    // exit: a suite run used to end without the summary, so a leak in a `main`-less guard
    // was invisible to the one instrument built to see it (loft#1357).  No-op unarmed.
    crate::state::text_timeline_summary();
    if total_fail == 0 {
        println!(
            "test result: ok. {total_pass} passed; {total_files} file{}  {scope}",
            if total_files == 1 { "" } else { "s" }
        );
        0
    } else {
        println!(
            "test result: FAILED. {total_fail} failed; {total_pass} passed; {total} total; {total_files} file{}  {scope}",
            if total_files == 1 { "" } else { "s" }
        );
        1
    }
}
