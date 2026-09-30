// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 01 — `loft introspect` CLI regression tests.
//!
//! Invokes the compiled binary via `std::process::Command` and asserts the
//! acceptance shapes from `plans/12-repl-and-introspection/01-introspection-cli.md`:
//! all-sections default, single-section selection, the default-stdlib filter
//! (the bug where the bytecode section leaked the whole stdlib), `--all-fns`,
//! and the `--fn` filter.  Assertion-based rather than byte-exact golden, so a
//! harmless codegen/format shift doesn't force a golden re-bless.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture() -> std::path::PathBuf {
    workspace_root().join("tests/data/introspect_golden.loft")
}

/// Run `loft introspect <args> <fixture>` and return (stdout, success).
fn introspect(args: &[&str]) -> (String, bool) {
    let out = Command::new(loft_bin())
        .arg("introspect")
        .args(args)
        .arg(fixture())
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

/// The text of one `=== <name> ===` section, up to the next `=== ` header.
fn section<'a>(stdout: &'a str, name: &str) -> &'a str {
    let header = format!("=== {name} ===");
    let start = stdout
        .find(&header)
        .unwrap_or_else(|| panic!("no `{header}` in:\n{stdout}"));
    let after = start + header.len();
    let rest = &stdout[after..];
    let end = rest
        .find("\n=== ")
        .map(|e| after + e)
        .unwrap_or(stdout.len());
    &stdout[after..end]
}

/// Acceptance #1 — default emits all four sections, in order, with the user fns.
#[test]
fn default_emits_all_four_sections() {
    let (out, ok) = introspect(&[]);
    assert!(ok, "introspect should exit 0; stdout:\n{out}");
    let (mut last, mut order) = (0usize, Vec::new());
    for sec in [
        "=== bytecode ===",
        "=== rust ===",
        "=== slots ===",
        "=== types ===",
    ] {
        let at = out
            .find(sec)
            .unwrap_or_else(|| panic!("missing {sec} in:\n{out}"));
        order.push(sec);
        assert!(
            at >= last,
            "sections out of order: {sec} before a prior one\n{out}"
        );
        last = at;
    }
    assert!(out.contains("n_dbl"), "user fn n_dbl missing\n{out}");
    assert!(out.contains("n_test"), "user fn n_test missing\n{out}");
}

/// Regression for the default-stdlib leak: the bytecode + slots + types
/// sections must show ONLY user functions, not the stdlib or the
/// compiler-synthesized runtime helpers (`i_parse_*`, `__iface_*`).
#[test]
fn default_filters_stdlib_and_internals() {
    let (out, ok) = introspect(&[]);
    assert!(ok);
    for name in ["bytecode", "slots", "types"] {
        let sec = section(&out, name);
        assert!(
            !sec.contains("i_parse_errors") && !sec.contains("__iface_"),
            "the `{name}` section leaked stdlib/internal fns (default filter broke):\n{sec}"
        );
        assert!(
            sec.contains("n_dbl"),
            "`{name}` section missing user fn n_dbl:\n{sec}"
        );
    }
}

/// Acceptance #2 — `--show-bytecode` emits ONLY the bytecode section.
#[test]
fn single_section_excludes_others() {
    let (out, ok) = introspect(&["--show-bytecode"]);
    assert!(ok);
    assert!(out.contains("n_dbl"), "bytecode of n_dbl missing\n{out}");
    for other in ["=== rust ===", "=== slots ===", "=== types ==="] {
        assert!(
            !out.contains(other),
            "single-section output leaked {other}\n{out}"
        );
    }
}

/// Acceptance #5 — `--all-fns` pulls the stdlib back into the bytecode section.
#[test]
fn all_fns_includes_stdlib() {
    let (out, ok) = introspect(&["--all-fns", "--show-bytecode"]);
    assert!(ok);
    assert!(
        out.contains("__iface_") || out.contains("i_parse"),
        "--all-fns should include stdlib/internal fns\n{}",
        &out[..out.len().min(2000)]
    );
}

/// Acceptance #4 — `--fn n_dbl` restricts a section to the named function.
#[test]
fn fn_filter_restricts_to_one() {
    let (out, ok) = introspect(&["--fn", "n_dbl", "--show-slots"]);
    assert!(ok);
    assert!(out.contains("n_dbl"), "filtered fn n_dbl missing\n{out}");
    assert!(
        !out.contains("n_test"),
        "--fn n_dbl should exclude n_test\n{out}"
    );
}

/// INSP.J — `--json` emits ONE machine-readable object over the sections,
/// parseable by loft's OWN JSON reader (dogfood), one string field per included
/// section, in canonical order.
#[test]
fn json_mode_emits_a_parseable_section_object() {
    use loft::json::Parsed;
    let (out, ok) = introspect(&["--json"]);
    assert!(ok, "introspect --json should exit 0; stdout:\n{out}");
    let doc = loft::json::parse(&out).expect("--json output is valid JSON (loft's own reader)");
    let Parsed::Object(fields) = &doc else {
        panic!("--json output is a JSON object, got: {doc:?}");
    };
    let keys: Vec<&str> = fields.iter().map(|(k, _, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        vec!["bytecode", "rust", "slots", "types"],
        "the four default sections, in canonical order"
    );
    // Each section value is a string carrying the same dump as the text mode.
    let types = fields
        .iter()
        .find_map(|(k, _, v)| (k == "types").then_some(v))
        .unwrap();
    let Parsed::Str(types_text) = types else {
        panic!("a section value is a JSON string, got {types:?}");
    };
    assert!(
        types_text.contains("n_dbl"),
        "the types section carries the user fn:\n{types_text}"
    );
}

/// `--json --show-bytecode` restricts the object to the one requested section.
#[test]
fn json_mode_respects_section_selection() {
    use loft::json::Parsed;
    let (out, ok) = introspect(&["--json", "--show-bytecode"]);
    assert!(ok);
    let doc = loft::json::parse(&out).expect("valid JSON");
    let Parsed::Object(fields) = &doc else {
        panic!("object, got {doc:?}");
    };
    let keys: Vec<&str> = fields.iter().map(|(k, _, _)| k.as_str()).collect();
    assert_eq!(keys, vec!["bytecode"], "only the requested section");
}

// ---------------------------------------------------------------------------
// @PLN103 — the `--show-ownership` overlay + `LOFT_STORES=timeline` summary.
// Assertion-based (not byte-golden): each check pins one SEMANTIC invariant per
// fact-kind, so a harmless var-number/format shift doesn't force a re-bless, but a
// regression in the ownership verdict (e.g. reverting the per-binding classification)
// turns a `Borrowed`/`Join` back into `Owned` and fails.
// ---------------------------------------------------------------------------

/// Run `introspect --show-ownership <flags> tests/data/ownership_corpus.loft`.
fn ownership(flags: &[&str]) -> String {
    let corpus = workspace_root().join("tests/data/ownership_corpus.loft");
    let out = Command::new(loft_bin())
        .arg("introspect")
        .arg("--show-ownership")
        .args(flags)
        .arg(&corpus)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    assert!(
        out.status.success(),
        "introspect --show-ownership should exit 0; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The overlay renders every invisible-fact-kind correctly on the committed IR.
#[test]
fn show_ownership_renders_each_fact_kind() {
    let out = ownership(&[]);
    // The verdict is backend-shared (P2.0) — the overlay says so.
    assert!(
        out.contains("backend-shared"),
        "missing the backend-shared note:\n{out}"
    );
    // K1 — a borrowed field projection is a genuine alias of its source param.
    assert!(
        out.contains("_mv_items_1") && out.contains("Borrowed(base=e)"),
        "K1: borrowed field projection not flagged as Borrowed(base=e):\n{out}"
    );
    // K2 — a whole-value bind COPIES (Owned); a projection read is a VIEW (Borrowed).
    let k2 = section_fn(&out, "n_k2_copy_vs_view");
    assert!(
        k2.contains("\n  1         b                      Owned"),
        "K2: `b = src` should be an Owned copy:\n{k2}"
    );
    assert!(
        k2.contains("first") && k2.contains("Borrowed(base="),
        "K2: `b[0]` should be a Borrowed view:\n{k2}"
    );
    // K4 — the empty `[]` arm is a REAL owned vector (the #562 fix), not a bare null;
    // and the return delivers via the return buffer.
    let k4 = section_fn(&out, "n_k4_emptyarm");
    // The FACT is the owned fresh store, not the temp's name: loft#699 routes an empty
    // `[]` through the same construction path a non-empty one takes, for every element
    // type rather than only the narrow-integer ones, so the accumulator is the ordinary
    // `_vec_N` instead of a shape-specific temp. Pinning the name would re-break here
    // for a rename that changes nothing about what the arm owns.
    assert!(
        k4.contains("Owned (backing="),
        "K4: empty-arm should own a fresh store:\n{k4}"
    );
    assert!(
        k4.contains("delivery:") && k4.contains("materialised"),
        "K4: return delivery should be materialised:\n{k4}"
    );
    // K1 runtime JOIN — owned on one path, a borrowed view on the other; scalars elided.
    let kj = section_fn(&out, "n_k1_owned_or_borrow");
    assert!(
        kj.contains("Join(base=pool)"),
        "K1-join: `r`/return should be Join(base=pool):\n{kj}"
    );
    assert!(
        kj.contains("(scalar)"),
        "scalars should render `— (scalar)`:\n{kj}"
    );
}

/// The overlay is deterministic (two runs identical) — safe to diff/golden.
#[test]
fn show_ownership_is_deterministic() {
    assert_eq!(ownership(&[]), ownership(&[]));
}

/// `LOFT_STORES=timeline` distinguishes a large WORKING SET from a leak — the
/// disambiguation `=warn` cannot make. A clean program: peak > 0, `NO leak`.
#[test]
fn timeline_summary_reports_working_set_no_leak() {
    let corpus = workspace_root().join("tests/data/ownership_corpus.loft");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&corpus)
        .env("LOFT_STORES", "timeline")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("[timeline] SUMMARY:") && err.contains("(working set)"),
        "missing timeline SUMMARY:\n{err}"
    );
    assert!(
        err.contains("NO leak"),
        "a clean program should report NO leak:\n{err}"
    );
    // Stable ids: at least one `alloc #<nr>.<seq>` with a dotted seq.
    assert!(
        err.contains("[timeline] alloc #") && err.contains('.'),
        "missing stable per-store ids:\n{err}"
    );
}

/// The text of one function's `--show-ownership` block, up to the next `fn ` header.
fn section_fn<'a>(stdout: &'a str, fn_name: &str) -> &'a str {
    let header = format!("fn {fn_name} ");
    let start = stdout
        .find(&header)
        .unwrap_or_else(|| panic!("no `{header}` in:\n{stdout}"));
    let rest = &stdout[start + header.len()..];
    let end = rest
        .find("\nfn ")
        .map_or(stdout.len(), |e| start + header.len() + e);
    &stdout[start..end]
}

/// Regression gate for BOTH captured-group UAF fixes (`plans/captured-group-elem-uaf.md`):
/// the 35m materialisation fix (vector-match text arm byte-copies into an owned buffer)
/// and the 35c fix (`collect_return_sources` skips trailing frees, so a returned enum
/// record is freed with `OpFreeRefIfDistinct`, not a plain `OpFreeRef`). Neither
/// `--show-ownership` UAF overlay may fire on the fixture (`bad`, `good`, `parse`); if
/// either fix regresses, its overlay fires and this test catches it. Each overlay's own
/// firing is proved parser-free in `use_analysis::{uaf_overlay_tests, return_source_tests}`,
/// so this gate does not need to reproduce the bugs.
#[test]
fn ownership_overlay_silent_after_captured_group_fix() {
    let out = Command::new(loft_bin())
        .arg("introspect")
        .arg("--show-ownership")
        .arg(workspace_root().join("tests/data/uaf_overlay.loft"))
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "introspect failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let count = stdout.matches("⚠ UAF").count();
    assert_eq!(
        count, 0,
        "materialisation fix regressed — free-before-dependent-read overlay fired:\n{stdout}"
    );
}

/// CLI error — a missing input file exits non-zero.
#[test]
fn missing_file_errors() {
    let out = Command::new(loft_bin())
        .arg("introspect")
        .arg("/no/such/introspect_input.loft")
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft");
    assert!(!out.status.success(), "missing file should exit non-zero");
}

// ---------------------------------------------------------------------------
// `--show-resolution` / `--why` — which names each source can SEE.
//
// The state these report decides whether an unqualified name resolves, and it
// used to be inspectable only by adding an `eprintln!` to the parser.  The
// assertions pin the three facts DEBUG_INTROSPECT.md § `--show-resolution` teaches a reader
// to read, so the doc and the output cannot drift apart:
//   1. the `context:` line (which stdlib and `--lib` paths this run searched),
//   2. `defined` vs `visible` per source,
//   3. one alias line per imported name.
// ---------------------------------------------------------------------------

/// Run `introspect --show-resolution` over a program that `use`s a library, so
/// there is a real import alias to report.  `tests/lib/typeshift` is the same
/// one-function fixture the session tests use.
fn resolution(args: &[&str]) -> String {
    // One file PER CALL, not per process.  Keyed on the pid alone, all four call
    // sites here shared a single path: the tests run on 8 threads, so one call's
    // `remove_file` below landed while another's `loft introspect` was still opening
    // it.  The subprocess then printed nothing and `section()` panicked with "no
    // `=== resolution ===` in:" — a ~50 % failure that passes in isolation, because
    // it needs a second caller in flight.  The counter makes each call's path its
    // own; the pid still separates concurrent `cargo test` invocations.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let prog = std::env::temp_dir().join(format!("loft_res_{}_{n}.loft", std::process::id()));
    std::fs::write(
        &prog,
        "use typeshift::*;\nfn main() { v = ts_touch(); assert(v == 7, \"lib\") }\n",
    )
    .expect("write temp program");
    let lib = workspace_root().join("tests/lib");
    let out = Command::new(loft_bin())
        .arg("introspect")
        .args(args)
        .arg("--lib")
        .arg(&lib)
        .arg(&prog)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&prog);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn resolution_section_reports_context_sources_and_aliases() {
    let out = resolution(&["--show-resolution"]);
    let sec = section(&out, "resolution");
    // 1. The context this run assembled.  An empty `lib_dirs` here under a `--lib`
    //    invocation is a whole class of bug (the flag never reached the session),
    //    which is why the line is printed even when everything works.
    assert!(
        sec.contains("context:") && sec.contains("lib_dirs=["),
        "the context line names the searched paths: {sec}"
    );
    assert!(
        !sec.contains("lib_dirs=[]"),
        "`--lib` was passed, so it must appear in the context: {sec}"
    );
    // 2. Per-source counts.
    assert!(
        sec.contains("sources:") && sec.contains("defined") && sec.contains("visible"),
        "per-source defined/visible counts: {sec}"
    );
    // 3. The import alias — the fact a `use` adds, and the one a rebuild that
    //    cannot reproduce its derived state destroys.
    assert!(
        sec.contains("import binding"),
        "a program with a `use` must report at least one alias: {sec}"
    );
    assert!(
        sec.contains("n_ts_touch"),
        "the imported function is the alias reported: {sec}"
    );
}

#[test]
fn why_reports_where_a_name_is_defined_and_reachable_from() {
    let out = resolution(&["--why", "ts_touch"]);
    let sec = section(&out, "resolution");
    assert!(
        sec.contains("`ts_touch` is #") && sec.contains("defined in source"),
        "`--why` names the definition: {sec}"
    );
    // Reachable from BOTH its own source and the importing one — the distinction
    // the whole section exists to make.
    assert!(
        sec.contains("(its own)") && sec.contains("(import alias)"),
        "`--why` separates own-source visibility from an import alias: {sec}"
    );
    // The control: a name nothing defines must say so rather than inventing a
    // location, since "not defined anywhere" is itself the answer to "why can't I
    // call this".
    let missing = resolution(&["--why", "no_such_function_anywhere"]);
    assert!(
        section(&missing, "resolution").contains("is not defined in any source"),
        "an unknown name is reported as such: {missing}"
    );
}

#[test]
fn resolution_section_is_available_as_json() {
    use loft::json::Parsed;
    let out = resolution(&["--json", "--show-resolution"]);
    let doc = loft::json::parse(&out).expect("valid JSON");
    let Parsed::Object(fields) = &doc else {
        panic!("object, got {doc:?}");
    };
    let keys: Vec<&str> = fields.iter().map(|(k, _, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        vec!["resolution"],
        "only the requested section, under the documented key"
    );
}

/// loft#750 — compiling one file TWICE with one binary must produce the same
/// bytecode and the same stack slots.
///
/// It did not: `store_confinement` answered a `HashMap`, and its caller
/// relocates each confined `__vdb`'s null-init — a relocation that cannot reach
/// its block puts the init back at body position 0, so visiting several
/// confined stores in Rust's per-process hash order PERMUTED the null-inits at
/// the head of the body, which moved the slots under them.
///
/// Program OUTPUT was stable throughout (the reordered declarations are
/// independent), so nothing computed a wrong answer. The cost was elsewhere: a
/// `--native` artifact whose source varies per process cannot be bit
/// reproducible (#711), and "prove this change emits byte-identical IR" — the
/// standing gate for every inert-first plan step — cannot tell "my change did
/// nothing" from "the hash seed moved". Over the script corpus, 23 of 599 files
/// differed from THEMSELVES; the native-emitter half fixed 20, these three are
/// the parser/`scopes` half.
///
/// Each file is compiled in TWO SEPARATE PROCESSES, which is what varies the
/// hash seed — two compilations inside one process would share it and pass
/// while the bug was live.
#[test]
fn compiling_the_same_file_twice_gives_the_same_bytecode_and_slots() {
    // All three are lambda / closure-capture shaped, which is where a confined
    // block sits inside a `map`/`filter` body or a short-lambda capture — the
    // shape whose relocation cannot reach its block and so puts the null-init
    // back, which is what makes the visit order observable.
    for name in [
        "tests/scripts/35p-iterator-match.loft",
        "tests/scripts/501-map-filter-literal-receiver.loft",
        "tests/scripts/85-short-lambda-capture.loft",
    ] {
        let run = || -> String {
            let out = Command::new(loft_bin())
                .arg("introspect")
                .arg(workspace_root().join(name))
                .current_dir(workspace_root())
                // This test is about COMPILATION being reproducible (loft#750), so both runs
                // have to compile.  The whole-program cache is default-on, and without this
                // the second invocation is a warm bundle load rather than a second compile —
                // it renders work-ref slots as the unassigned `u16::MAX` and the comparison
                // reports a reproducibility failure that is really a cold-vs-warm difference.
                .env("LOFT_NO_CACHE", "1")
                .output()
                .expect("failed to invoke loft binary");
            assert!(out.status.success(), "introspect {name} failed");
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        let first = run();
        let second = run();
        assert!(
            !first.is_empty(),
            "{name}: empty introspect output — the comparison below would be vacuous"
        );
        if first != second {
            let diff: Vec<String> = first
                .lines()
                .zip(second.lines())
                .filter(|(a, b)| a != b)
                .take(6)
                .map(|(a, b)| format!("  - {a}\n  + {b}"))
                .collect();
            panic!(
                "{name}: two compilations with one binary disagree — compilation \
                 is not reproducible (loft#750). First differing lines:\n{}",
                diff.join("\n")
            );
        }
    }
}

/// `OpSetStackRef(r, v1)` declares no `fld` operand, so nothing may follow it in the stream but
/// the next op.  The `&`-link write wrote the `0u16` field offset its sibling ops declare behind
/// it too, and those two bytes ran as `OpGoto(+0)` — harmless only because `OpGoto` is opcode 0,
/// and counted by the operator census as a live `OpGoto`.
#[test]
fn a_set_stack_ref_is_followed_by_the_next_op() {
    let file = workspace_root().join("tests/data/set_stack_ref.loft");
    let out = Command::new(loft_bin())
        .args(["introspect", "--show-bytecode"])
        .arg(&file)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let listing = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = listing.lines().collect();
    let sites: Vec<usize> = (0..lines.len())
        .filter(|&i| lines[i].contains("SetStackRef("))
        .collect();
    assert!(
        sites.len() >= 2,
        "the fixture no longer writes through OpSetStackRef:\n{listing}"
    );
    for i in sites {
        let next = lines.get(i + 1).copied().unwrap_or("");
        assert!(
            !next.contains("Goto("),
            "operand bytes behind OpSetStackRef decode as an op:\n{}\n{next}",
            lines[i]
        );
    }
    let run = Command::new(loft_bin())
        .args(["--interpret"])
        .arg(&file)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
