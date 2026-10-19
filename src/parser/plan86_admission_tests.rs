// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

use super::Parser;
use crate::sandbox::{CapViolation, TotalityViolation, parse_sandbox_config};

fn quoted(items: &[&str]) -> String {
    items
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Load the stdlib + parse `src` under the literal `[sandbox]` config `cfg`.
fn parse_admit_cfg(cfg: &str, src: &str) -> Parser {
    let mut p = Parser::new();
    p.set_sandbox_config(parse_sandbox_config(cfg));
    p.parse_dir("default", true, true).unwrap();
    // A process-global counter, not the `src` pointer: cargo runs tests as
    // threads in ONE process, so a deterministic name (the literal's address)
    // can collide across concurrent threads — Windows' strict file locking
    // then fails the write where Unix tolerates it.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "plan86_admit_{}_{}.loft",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));
    crate::file_access::write(&path, src).unwrap();
    p.parse(path.to_str().unwrap(), false);
    let _ = crate::file_access::remove_file(&path);
    p
}

/// #631 — parse `src` from a temp file whose BASENAME is handed to `cfg_for`, so
/// a test can designate the file by a path selector and allow-list the library
/// the source actually lands in (both are derived from the generated name).
fn parse_admit_file(cfg_for: impl Fn(&str, &str) -> String, src: &str) -> Parser {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let stem = format!(
        "plan86file_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    );
    let path = std::env::temp_dir().join(format!("{stem}.loft"));
    let mut p = Parser::new();
    // `def_library` is the file stem, so stem doubles as the library name.
    p.set_sandbox_config(parse_sandbox_config(&cfg_for(
        &format!("{stem}.loft"),
        &stem,
    )));
    p.parse_dir("default", true, true).unwrap();
    crate::file_access::write(&path, src).unwrap();
    p.parse(path.to_str().unwrap(), false);
    let _ = crate::file_access::remove_file(&path);
    p
}

/// #631 — an entry point that fills its result through a private helper.  The
/// mutation escapes `build`, so it is the raw write the rule exists to reject;
/// moving it into `push` is the laundering.
const LAUNDER_SRC: &str = "struct S { v: vector<integer> }\n\
                           fn push(s: S, x: integer) { s.v += [x]; }\n\
                           fn build(n: integer) -> S { out = S { v: [] }; \
                           for i in 0..n { push(out, i); } return out }\n";

fn parse_admit_libs(
    designations: &[&str],
    allow_libs: &[&str],
    allow: &[&str],
    src: &str,
) -> Parser {
    let cfg = format!(
        "[sandbox]\nmod = [{}]\n[profile.mod]\nallow_libs = [{}]\nallow = [{}]\n",
        quoted(designations),
        quoted(allow_libs),
        quoted(allow),
    );
    parse_admit_cfg(&cfg, src)
}

/// Parse `src` with a `mod` profile carrying data-envelope bounds (`0` = omit).
fn parse_admit_envelope(
    designations: &[&str],
    allow_libs: &[&str],
    data_budget: u64,
    max_input_n: u64,
    src: &str,
) -> Parser {
    let cfg = format!(
        "[sandbox]\nmod = [{}]\n[profile.mod]\nallow_libs = [{}]\ndata_budget = {}\n{}",
        quoted(designations),
        quoted(allow_libs),
        data_budget,
        if max_input_n == 0 {
            String::new()
        } else {
            format!("max_input_n = {max_input_n}\n")
        },
    );
    parse_admit_cfg(&cfg, src)
}

fn parse_admit(designations: &[&str], allow_caps: &[&str], src: &str) -> Parser {
    parse_admit_libs(designations, &[], allow_caps, src)
}

fn viol_symbol(v: &CapViolation) -> u32 {
    match v {
        CapViolation::UngrantedCap { symbol, .. }
        | CapViolation::UntaggedSymbol { symbol, .. }
        | CapViolation::ExternalFfi { symbol, .. }
        | CapViolation::CBinding { symbol, .. } => *symbol,
    }
}

/// #631 — `allow_libs` naming the library the sandboxed code ITSELF lives in is
/// rejected.  That policy makes every function in the module a trusted leaf, so
/// the raw-write guard stops seeing its helpers and the data envelope stops
/// counting their loops — both silently.  It is the policy the old diagnostic
/// suggested first, and it is the whole mechanism behind the laundering below.
#[test]
fn allow_listing_the_sandboxed_code_s_own_library_is_rejected() {
    let p = parse_admit_file(
        |file, lib| {
            format!(
                "[sandbox]\nmod = [\"fn:build\"]\n[profile.mod]\n\
                 allow_libs = [\"code\", \"{lib}\"]\n# file: {file}\n"
            )
        },
        LAUNDER_SRC,
    );
    let errors = p.sandbox_admission_errors();
    assert!(
        errors.iter().any(|e| e.contains("cannot vet itself")),
        "a self-allow-list must be rejected, got: {errors:?}"
    );
}

/// #631 — allow-listing a genuine HOST library stays admitted: the rejection
/// above must key on the library holding sandboxed code, not on `allow_libs`.
#[test]
fn allow_listing_a_host_library_is_still_admitted() {
    let p = parse_admit_file(
        |file, _lib| {
            format!("[sandbox]\nmod = [\"{file}\"]\n[profile.mod]\nallow_libs = [\"code\"]\n")
        },
        "fn helper(n: integer) -> integer { n + 1 }\n\
         fn entry(n: integer) -> integer { helper(n) }\n",
    );
    assert!(
        p.sandbox_admission_errors().is_empty(),
        "a host-library allow-list must stay admitted: {:?}",
        p.sandbox_admission_errors()
    );
}

/// #631 shape 1 — the laundering itself.  With the module's own file designated,
/// the escaping mutation is caught wherever it is written: moving `out.v += [i]`
/// into a one-line helper must NOT change the verdict.
#[test]
fn a_same_library_helper_cannot_launder_an_escaping_raw_write() {
    let cfg = |file: &str, _lib: &str| {
        format!("[sandbox]\nmod = [\"{file}\"]\n[profile.mod]\nallow_libs = [\"code\"]\n")
    };
    let laundered = parse_admit_file(cfg, LAUNDER_SRC);
    let inline = parse_admit_file(
        cfg,
        "struct S { v: vector<integer> }\n\
         fn build(n: integer) -> S { out = S { v: [] }; \
         for i in 0..n { out.v += [i]; } return out }\n",
    );
    assert!(
        !inline.sandbox_raw_writes().is_empty(),
        "the inline form is the control — it must reject"
    );
    assert!(
        !laundered.sandbox_raw_writes().is_empty(),
        "the same mutation moved into a same-library helper must reject too"
    );
}

/// #631 shape 2 — the data envelope must count a designated file's helpers.  An
/// entry point that delegates all its work reported `O(1)` while being `O(n)`, so
/// a declared `data_budget` could be satisfied by a plugin that does not fit it.
#[test]
fn complexity_counts_loops_in_a_designated_file_s_helpers() {
    let p = parse_admit_file(
        |file, _lib| {
            format!("[sandbox]\nmod = [\"{file}\"]\n[profile.mod]\nallow_libs = [\"code\"]\n")
        },
        "fn render(doc: vector<integer>) -> integer { t = 0; \
         for i in 0..len(doc) { t += doc[i] ?? 0; } return t }\n\
         fn dispatch(doc: vector<integer>) -> integer { return render(doc) }\n",
    );
    assert!(
        p.sandbox_admission_errors().is_empty(),
        "should admit: {:?}",
        p.sandbox_admission_errors()
    );
    assert_eq!(
        p.sandbox_complexity_degree(),
        1,
        "`dispatch` has no loop of its own but reaches one — that is O(n), not O(1)"
    );
}

/// #631 — reaching an own-library helper names the fix that WORKS (designate it)
/// and warns off the one that silently disables the guard (`allow_libs`).
#[test]
fn reaching_an_own_library_helper_suggests_designation_not_allow_libs() {
    let p = parse_admit_file(
        |_file, _lib| {
            "[sandbox]\nmod = [\"fn:build\"]\n[profile.mod]\nallow_libs = [\"code\"]\n".to_string()
        },
        LAUNDER_SRC,
    );
    let errors = p.sandbox_admission_errors();
    let msg = errors.join("\n");
    assert!(
        msg.contains("\"fn:push\"") && msg.contains("do NOT"),
        "the fix must lead with designation and warn off allow_libs, got: {msg}"
    );
}

/// @PLN86 2.3 — the admission convergence: a granted-cap reference admits, an
/// ungranted-cap reference is rejected naming the group, an untagged symbol is
/// rejected (deny-by-default).
#[test]
fn admission_grants_allowed_caps_and_rejects_ungranted_and_untagged() {
    let src = "fn cap_fs_read() -> integer fs#read;\n#native\n\
               fn cap_coll_read() -> integer collections#read;\n#native\n\
               fn cap_untagged() -> integer;\n#native\n\
               fn ok() -> integer { cap_coll_read() }\n\
               fn bad() -> integer { cap_fs_read() }\n\
               fn uses_untagged() -> integer { cap_untagged() }\n";
    let p = parse_admit(
        &["fn:ok", "fn:bad", "fn:uses_untagged"],
        &["collections#read"],
        src,
    );
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    let v = p.sandbox_admit();
    // bad reaches fs.read which is not granted → named violation.
    assert!(
        v.contains(&CapViolation::UngrantedCap {
            from: p.data.def_nr("n_bad"),
            symbol: p.data.def_nr("n_cap_fs_read"),
            group: "fs#read".to_string(),
        }),
        "expected UngrantedCap(fs.read), got {v:?}"
    );
    // uses_untagged reaches an unclassified symbol → deny-by-default.
    assert!(
        v.contains(&CapViolation::UntaggedSymbol {
            from: p.data.def_nr("n_uses_untagged"),
            symbol: p.data.def_nr("n_cap_untagged"),
        }),
        "expected UntaggedSymbol, got {v:?}"
    );
    // ok reaches only collections.read (granted) → admitted clean.
    let coll = p.data.def_nr("n_cap_coll_read");
    assert!(
        !v.iter().any(|viol| viol_symbol(viol) == coll),
        "granted collections.read must admit clean, got {v:?}"
    );
}

/// @PLN86 2.3 / L4 — an indirect call through a fn-ref cannot escape: passing
/// `cap_fs_read` to a higher-order fn still puts it in the reachable set, so
/// admission rejects the ungranted `fs.read` group.
#[test]
fn admission_rejects_indirect_fnref_call_l4() {
    let src = "fn cap_fs_read(n: integer) -> integer fs#read;\n#native\n\
               fn apply(f: fn(integer) -> integer, n: integer) -> integer { f(n) }\n\
               fn sneaky() -> integer { apply(cap_fs_read, 5) }\n";
    let p = parse_admit(&["fn:sneaky", "fn:apply"], &["collections#read"], src);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    let v = p.sandbox_admit();
    assert!(
        v.contains(&CapViolation::UngrantedCap {
            from: p.data.def_nr("n_sneaky"),
            symbol: p.data.def_nr("n_cap_fs_read"),
            group: "fs#read".to_string(),
        }),
        "L4 indirect fn-ref to fs.read must be rejected, got {v:?}"
    );
}

/// @PLN86 2.2 — the coverage lint lists an untagged public function and omits a
/// tagged one (the work-list for tagging the stdlib/library surface).
#[test]
fn coverage_lint_lists_untagged_public_functions() {
    let src = "pub fn tagged_fn() -> integer math#read;\n#native\n\
               pub fn untagged_fn() -> integer;\n#native\n";
    let p = parse_admit(&[], &[], src);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    let untagged = p.untagged_public_symbols();
    assert!(
        untagged.contains(&p.data.def_nr("n_untagged_fn")),
        "an untagged public fn must be listed"
    );
    assert!(
        !untagged.contains(&p.data.def_nr("n_tagged_fn")),
        "a tagged public fn must NOT be listed"
    );
}

/// @PLN86 2.2 — the REAL stdlib fs/env surface gates correctly: `mtime`
/// (tagged `fs.read`, bodiless so no untagged deps) is rejected naming the
/// group when fs.read is not granted, while `env_variable` (`env`, granted)
/// admits — and the tagged fs/env fns have left the coverage lint.
#[test]
fn stdlib_fs_env_caps_gate_real_functions() {
    let src = "fn reads_mtime() -> integer { mtime(\"x\") }\n\
               fn reads_env() -> text { env_variable(\"X\") }\n";
    let p = parse_admit(&["fn:reads_mtime", "fn:reads_env"], &["env#read"], src);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    // the fs/env surface is no longer flagged by the coverage lint
    let untagged = p.untagged_public_symbols();
    for n in [
        "n_content",
        "n_write",
        "n_env_variable",
        "n_mtime",
        "n_file",
    ] {
        assert!(
            !untagged.contains(&p.data.def_nr(n)),
            "{n} should be tagged (off the lint)"
        );
    }
    let v = p.sandbox_admit();
    // mtime (fs#read) is not granted → rejected naming the real group.
    assert!(
        v.contains(&CapViolation::UngrantedCap {
            from: p.data.def_nr("n_reads_mtime"),
            symbol: p.data.def_nr("n_mtime"),
            group: "fs#read".to_string(),
        }),
        "mtime must be rejected naming fs.read, got {v:?}"
    );
    // env_variable (env) is granted → admits clean.
    let envv = p.data.def_nr("n_env_variable");
    assert!(
        !v.iter().any(|x| viol_symbol(x) == envv),
        "granted env must admit, got {v:?}"
    );
}

/// @PLN86 — library-first admission: a wholesale-allowed library admits its
/// UNTAGGED functions with no `#cap` tag (the common "include a whole
/// library" case).  `now()` lives in the `files` stdlib module and is
/// untagged; under `allow_libs=["files"]` it admits, without it it is an
/// `UntaggedSymbol`.  Tags stay the fine-grained layer, not a requirement.
#[test]
fn wholesale_allowed_library_admits_untagged_functions() {
    // def_library identifies the stdlib module from the source file.
    let p0 = parse_admit(&[], &[], "fn ignore() -> integer { 1 }\n");
    assert_eq!(
        p0.def_library(p0.data.def_nr("n_mtime")).as_deref(),
        Some("files")
    );

    let src = "fn uses_now() -> integer { now() }\n";
    // (a) files allowed wholesale → untagged now() admits, no tag needed.
    let p = parse_admit_libs(&["fn:uses_now"], &["files"], &[], src);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    assert!(
        p.sandbox_admit().is_empty(),
        "wholesale files must admit untagged now(), got {:?}",
        p.sandbox_admit()
    );
    // (b) nothing allowed → now() is rejected (deny-by-default).
    let p2 = parse_admit_libs(&["fn:uses_now"], &[], &[], src);
    let now = p2.data.def_nr("n_now");
    assert!(
        p2.sandbox_admit().iter().any(|x| viol_symbol(x) == now),
        "without allow_libs, untagged now() must be flagged, got {:?}",
        p2.sandbox_admit()
    );
}

/// @PLN86 2.5 — admission errors are actionable: each class names the
/// position, the symbol, the rule, and BOTH fixes (wholesale lib + the
/// fine-grained cap / native_ffi).
#[test]
fn admission_errors_name_symbol_rule_and_fix() {
    // UngrantedCap — mtime needs fs#read; only env#read is granted.
    let p = parse_admit(
        &["fn:reads_mtime"],
        &["env#read"],
        "fn reads_mtime() -> integer { mtime(\"x\") }\n",
    );
    let errs = p.sandbox_admission_errors();
    assert_eq!(errs.len(), 1, "{errs:?}");
    let e = &errs[0];
    eprintln!("CAP_DIAG: {e}");
    assert!(e.contains("mtime"), "names the symbol: {e}");
    assert!(e.contains("fs#read"), "names the group: {e}");
    assert!(e.contains("fix:"), "points at the fix: {e}");
    assert!(
        e.contains("`allow`") && e.contains("allow_libs"),
        "offers both fixes: {e}"
    );
    assert!(e.contains(".loft:"), "carries a source position: {e}");

    // UntaggedSymbol — now() in `files`, nothing allowed.
    let p2 = parse_admit_libs(
        &["fn:uses_now"],
        &[],
        &[],
        "fn uses_now() -> integer { now() }\n",
    );
    assert!(
        p2.sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("now")
                && e.contains("library `files`")
                && e.contains("allow_libs")),
        "untagged error names the library + allow_libs: {:?}",
        p2.sandbox_admission_errors()
    );
}

/// @PLN86 2.5 — an external-FFI rejection names the crate and `native_ffi`.
#[test]
fn admission_error_for_external_ffi_names_crate() {
    let src = "fn ext_fn() -> integer; #native \"ext_sym\"\n\
               fn calls_ext() -> integer { ext_fn() }\n";
    let mut p = parse_admit(&["fn:calls_ext"], &[], src);
    p.data
        .native_symbol_crates
        .insert("ext_sym".to_string(), "extcrate".to_string());
    assert!(
        p.sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("ext_fn") && e.contains("extcrate") && e.contains("native_ffi")),
        "external-FFI error names the crate + native_ffi: {:?}",
        p.sandbox_admission_errors()
    );
}

/// @PLN86 3.1 — totality rejects an unbounded `while`, admits a bounded `for`.
#[test]
fn totality_rejects_while_admits_for() {
    // An UNBOUNDED `while` (a flag loop — no decreasing variant) is rejected;
    // bounded `while`s are admitted now and covered by `bounded_while_is_admitted`.
    let p = parse_admit_libs(
        &["fn:loops"],
        &["code"],
        &[],
        "fn loops(go: boolean) -> integer { x = 0; while go { x += 1 } x }\n",
    );
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    let t = p.sandbox_totality();
    assert!(
        t.iter()
            .any(|v| matches!(v, TotalityViolation::UnboundedLoop { .. })),
        "a `while` must be rejected, got {t:?}"
    );
    // the rendered error points at the bounded alternative
    assert!(
        p.sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("while") && e.contains("for ")),
        "{:?}",
        p.sandbox_admission_errors()
    );

    let p2 = parse_admit_libs(
        &["fn:counts"],
        &["code"],
        &[],
        "fn counts() -> integer { s = 0; for i in 0..10 { s += i } s }\n",
    );
    assert!(
        p2.sandbox_totality().is_empty(),
        "a bounded `for` must be total, got {:?}",
        p2.sandbox_totality()
    );
}

/// @PLN86 3.2 — totality rejects recursion (self + mutual), admits acyclic.
#[test]
fn totality_rejects_recursion_admits_acyclic() {
    // self-recursion `f -> f`
    let p = parse_admit_libs(
        &["fn:rec"],
        &["code"],
        &[],
        "fn rec(n: integer) -> integer { rec(n + 1) }\n",
    );
    assert!(
        p.sandbox_totality()
            .iter()
            .any(|v| matches!(v, TotalityViolation::Recursion { .. })),
        "self-recursion must be rejected, got {:?}",
        p.sandbox_totality()
    );
    assert!(
        p.sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("recursion") && e.contains("rec")),
        "{:?}",
        p.sandbox_admission_errors()
    );

    // mutual recursion `a -> b -> a`
    let p2 = parse_admit_libs(
        &["fn:a", "fn:b"],
        &["code"],
        &[],
        "fn a(n: integer) -> integer { b(n) }\nfn b(n: integer) -> integer { a(n) }\n",
    );
    assert!(
        p2.sandbox_totality()
            .iter()
            .any(|v| matches!(v, TotalityViolation::Recursion { .. })),
        "mutual recursion must be rejected, got {:?}",
        p2.sandbox_totality()
    );

    // acyclic call chain `top -> helper`
    let p3 = parse_admit_libs(
        &["fn:top", "fn:helper"],
        &["code"],
        &[],
        "fn helper(n: integer) -> integer { n + 1 }\n\
         fn top(n: integer) -> integer { helper(n) }\n",
    );
    assert!(
        !p3.sandbox_totality()
            .iter()
            .any(|v| matches!(v, TotalityViolation::Recursion { .. })),
        "an acyclic script must be total, got {:?}",
        p3.sandbox_totality()
    );
}

/// @PLN86 3.3 — total-op check: an explicit-abort op (`assert`) is excluded
/// (it faults the script), while arithmetic stays total — a divide-by-zero is
/// the interpreter's null sentinel, NOT a rejected op.
#[test]
fn totality_excludes_abort_ops_admits_total_arithmetic() {
    // assert faults → excluded as a partial op
    let p = parse_admit_libs(
        &["fn:checks"],
        &["code"],
        &[],
        "fn checks(n: integer) -> integer { assert(n > 0, \"pos\"); n }\n",
    );
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    assert!(
        p.sandbox_totality()
            .iter()
            .any(|v| matches!(v, TotalityViolation::PartialOp { .. })),
        "assert must be excluded as a partial op, got {:?}",
        p.sandbox_totality()
    );
    assert!(
        p.sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("assert") && e.contains("fix")),
        "{:?}",
        p.sandbox_admission_errors()
    );

    // a divide-by-zero is total on the interpreter → NOT a partial op
    let p2 = parse_admit_libs(
        &["fn:divides"],
        &["code"],
        &[],
        "fn divides(a: integer, b: integer) -> integer { a / b ?? 0 }\n",
    );
    assert!(
        !p2.sandbox_totality()
            .iter()
            .any(|v| matches!(v, TotalityViolation::PartialOp { .. })),
        "arithmetic is total (not excluded), got {:?}",
        p2.sandbox_totality()
    );
}

/// @PLN86 — any designated def flags the program as carrying sandboxed code (which
/// gates the load-time admission walk); a program with no sandboxed defs does not.
/// The flag is backend-agnostic — it no longer forces interpret-only.
#[test]
fn has_sandboxed_defs_on_any_designation() {
    let p = parse_admit_libs(
        &["fn:scripted"],
        &["code"],
        &[],
        "fn scripted() -> integer { 1 }\n",
    );
    assert!(
        p.has_sandboxed_defs(),
        "a designated def must flag the program as sandboxed"
    );
    let p2 = parse_admit_libs(&[], &["code"], &[], "fn plain() -> integer { 1 }\n");
    assert!(!p2.has_sandboxed_defs(), "no sandboxed defs → not flagged");
}

/// @PLN86 3.4 — the worst-case complexity degree counts loop nesting, and
/// composes across the acyclic call graph (a loop calling a looping fn → n²).
#[test]
fn complexity_degree_counts_loop_nesting_inter_procedural() {
    // no loop → O(1)
    let p0 = parse_admit_libs(
        &["fn:flat"],
        &["code"],
        &[],
        "fn flat() -> integer { 1 + 2 }\n",
    );
    assert_eq!(p0.sandbox_complexity_degree(), 0, "no loops → O(1)");

    // one loop → O(n)
    let p1 = parse_admit_libs(
        &["fn:one"],
        &["code"],
        &[],
        "fn one() -> integer { s = 0; for i in 0..10 { s += i } s }\n",
    );
    assert_eq!(p1.sandbox_complexity_degree(), 1, "one loop → O(n)");

    // nested loops → O(n^2)
    let p2 = parse_admit_libs(
        &["fn:nest"],
        &["code"],
        &[],
        "fn nest() -> integer { s = 0; for i in 0..10 { for j in 0..10 { s += i } } s }\n",
    );
    assert_eq!(p2.sandbox_complexity_degree(), 2, "nested loops → O(n^2)");
    assert!(
        p2.sandbox_complexity_report().contains("O(n^2)"),
        "{}",
        p2.sandbox_complexity_report()
    );

    // inter-procedural: a loop calling a looping fn → O(n^2)
    let p3 = parse_admit_libs(
        &["fn:outer", "fn:inner"],
        &["code"],
        &[],
        "fn inner() -> integer { s = 0; for j in 0..10 { s += j } s }\n\
         fn outer() -> integer { s = 0; for i in 0..10 { s += inner() } s }\n",
    );
    assert_eq!(
        p3.sandbox_complexity_degree(),
        2,
        "a loop calling a looping fn → O(n^2)"
    );
}

/// @PLN86 space budget — the SPACE degree counts only ACCUMULATING appends (a
/// structure grown across a loop), so a pure-compute loop is O(1) space even
/// when it is O(n) time, a transient buffer (reset each iteration) is O(1), and
/// a vector built across nested loops is O(n^2).
#[test]
fn space_degree_counts_accumulating_appends_only() {
    // pure compute — O(n) TIME, O(1) SPACE (no heap grows)
    let p0 = parse_admit_libs(
        &["fn:spc_pure"],
        &["code"],
        &[],
        "fn spc_pure() -> integer { s = 0; for i in 0..10 { s += i } s }\n",
    );
    assert_eq!(p0.sandbox_complexity_degree(), 1, "time O(n)");
    assert_eq!(p0.sandbox_space_degree(), 0, "pure compute → O(1) space");

    // accumulate into a vector declared OUTSIDE the loop → O(n) SPACE
    let p1 = parse_admit_libs(
        &["fn:spc_build"],
        &["code"],
        &[],
        "fn spc_build() -> integer { r = []; for i in 0..10 { r += [i] } len(r) }\n",
    );
    assert_eq!(
        p1.sandbox_space_degree(),
        1,
        "accumulating vector → O(n) space"
    );

    // transient buffer — reset every iteration → O(1) SPACE
    let p2 = parse_admit_libs(
        &["fn:spc_trans"],
        &["code"],
        &[],
        "fn spc_trans() -> integer { c = 0; for i in 0..10 { b = []; b += [i]; c += len(b) } c }\n",
    );
    assert_eq!(
        p2.sandbox_space_degree(),
        0,
        "a buffer reset each iteration is transient → O(1) space, got {}",
        p2.sandbox_space_degree()
    );

    // accumulate across NESTED loops → O(n^2) SPACE; report names both axes
    let p3 = parse_admit_libs(
        &["fn:spc_grid"],
        &["code"],
        &[],
        "fn spc_grid() -> integer { r = []; for i in 0..10 { for j in 0..10 { r += [i] } } len(r) }\n",
    );
    assert_eq!(
        p3.sandbox_space_degree(),
        2,
        "nested accumulation → O(n^2) space"
    );
    assert!(
        p3.sandbox_complexity_report().contains("space O(n^2)"),
        "report names the space axis: {}",
        p3.sandbox_complexity_report()
    );
}

/// @PLN86 P7.1 (F9) — the space footprint reports `(degree, coeff)`: a per-element
/// build loop accumulates ONE record's stride per appended element.
#[test]
fn space_footprint_reports_degree_and_record_coefficient() {
    // vector<integer>: one i64 element = 8 bytes per append.
    let pi = parse_admit_libs(
        &["fn:b"],
        &["code"],
        &[],
        "fn b() -> integer { r = []; for i in 0..10 { r += [i] } len(r) }\n",
    );
    assert_eq!(
        pi.sandbox_space_footprint(),
        (1, 8),
        "vector<integer> build → (degree 1, coeff 8)"
    );

    // vector<Mob>: coeff = the struct's record stride (one i64 field = 8 bytes).
    let ps = parse_admit_libs(
        &["fn:bs"],
        &["code"],
        &[],
        "struct Mob { hp: integer }\n\
         fn bs() -> integer { acc: vector<Mob> = []; \
          for i in 0..10 { acc += [Mob { hp: i }] } len(acc) }\n",
    );
    assert!(
        ps.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        ps.diagnostics.lines()
    );
    // vector<Mob> stores DbRef slots (a reference element) → sizeof(DbRef)=12 bytes
    // per element in the backing (the Mob record body is a separate allocation, a
    // documented v1 under-count). Degree 1, coeff 12 (one slot per appended element).
    assert_eq!(
        ps.sandbox_space_footprint(),
        (1, 12),
        "vector<Mob> build → (degree 1, coeff 12 = DbRef backing slot)"
    );
}

/// @PLN86 §8 (F11) — the data-envelope reject: `coeff · max_input_n^degree` must
/// fit `data_budget`, with `max_input_n` provable; over-budget / unprovable reject.
#[test]
fn data_budget_rejects_over_envelope_and_admits_under() {
    // degree 1, coeff 8 (vector<integer>); figure = 8 · max_input_n.
    let src = "fn build() -> integer { r = []; for i in 0..10 { r += [i] } len(r) }\n";

    // over budget: 8 · 1000 = 8000 > 4000 → rejected, naming the figure + budget.
    let over = parse_admit_envelope(&["fn:build"], &["code"], 4000, 1000, src);
    let errs = over.sandbox_admission_errors();
    assert!(
        errs.iter()
            .any(|e| e.contains("8000") && e.contains("data_budget 4000")),
        "over-budget must be rejected naming the figure: {errs:?}"
    );

    // under budget: 8 · 1000 = 8000 ≤ 40000 → admitted (no data-envelope error).
    let under = parse_admit_envelope(&["fn:build"], &["code"], 40000, 1000, src);
    assert!(
        !under
            .sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("data_budget") || e.contains("peak heap")),
        "under-budget must admit: {:?}",
        under.sandbox_admission_errors()
    );

    // budget set but max_input_n unset, degree > 0 → unprovable → rejected.
    let unprov = parse_admit_envelope(&["fn:build"], &["code"], 40000, 0, src);
    assert!(
        unprov
            .sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("cannot be bounded") && e.contains("max_input_n")),
        "an unset max_input_n with a growing footprint must be rejected: {:?}",
        unprov.sandbox_admission_errors()
    );

    // no data_budget → report-only (the same growing script admits).
    let nobudget = parse_admit_envelope(&["fn:build"], &["code"], 0, 0, src);
    assert!(
        !nobudget
            .sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("peak heap")),
        "no data_budget → report-only, no reject: {:?}",
        nobudget.sandbox_admission_errors()
    );
}

/// @PLN86 §8 (F10) — under an active envelope, an uncapped dynamic string build is
/// rejected (its bytes aren't a fixed record stride); a `max_string_len` cap admits.
#[test]
fn unbounded_string_build_rejected_unless_capped() {
    // grows `s` across a loop; max_input_n set so ONLY the string gate can fire.
    let src = "fn grow() -> text { s = \"\"; for i in 0..10 { s += \"x\" } s }\n";
    let base = "[sandbox]\nmod = [\"fn:grow\"]\n[profile.mod]\n\
                allow_libs = [\"code\"]\ndata_budget = 1000000\nmax_input_n = 100\n";

    // uncapped (no max_string_len) → UnboundedAlloc rejection.
    let uncapped = parse_admit_cfg(base, src);
    assert!(
        uncapped.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        uncapped.diagnostics.lines()
    );
    let errs = uncapped.sandbox_admission_errors();
    assert!(
        errs.iter()
            .any(|e| e.contains("string grows unboundedly") && e.contains("max_string_len")),
        "an uncapped string build must be rejected: {errs:?}"
    );

    // capped → admits (the cap bounds it).
    let capped_cfg = format!("{base}max_string_len = 256\n");
    let capped = parse_admit_cfg(&capped_cfg, src);
    assert!(
        !capped
            .sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("string grows unboundedly")),
        "a max_string_len-capped string build must admit: {:?}",
        capped.sandbox_admission_errors()
    );
}

/// @PLN86 prevention #3 — TOTAL host capabilities: a `#cap`-tagged function that
/// can reach an abort op (directly OR via a helper) is flagged as not total; a
/// capability that validates and returns a clean value is clean.  The host-side
/// mirror of the script-side 3.3 abort-op exclusion.
#[test]
fn capability_totality_flags_abort_reaching_caps() {
    let src = "\
        fn cap_bad(n: integer) -> integer game#read { assert(n > 0, \"pos\"); n }\n\
        fn cap_ok(n: integer) -> integer game#read { if n > 0 { n } else { 0 } }\n\
        fn helper(n: integer) -> integer { assert(n > 0, \"x\"); n }\n\
        fn cap_trans(n: integer) -> integer game#read { helper(n) }\n";
    let p = parse_admit_libs(&[], &["code"], &[], src);
    let v = p.sandbox_capability_totality_violations();
    let flagged: std::collections::HashSet<&str> =
        v.iter().map(|x| p.data.def(x.capability).name()).collect();
    // direct abort + transitive abort (via a non-cap helper) are both caught
    assert!(
        flagged.contains("n_cap_bad"),
        "cap_bad reaches assert: {flagged:?}"
    );
    assert!(
        flagged.contains("n_cap_trans"),
        "cap_trans reaches assert via helper: {flagged:?}"
    );
    // a total capability is NOT flagged; a non-cap helper is never enumerated
    assert!(
        !flagged.contains("n_cap_ok"),
        "cap_ok is total: {flagged:?}"
    );
    assert!(
        !flagged.contains("n_helper"),
        "non-cap helper is not a capability: {flagged:?}"
    );
    // the message is actionable
    let bad = v
        .iter()
        .find(|x| p.data.def(x.capability).name() == "n_cap_bad")
        .unwrap();
    let msg = crate::sandbox::describe_cap_totality_violation(&p.data, bad);
    assert!(
        msg.contains("not total") && msg.contains("cap_bad"),
        "actionable message: {msg}"
    );
}

/// @PLN86 prevention #4 — adversarial admission ESCAPE suite.  A battery that
/// TRIES to break out across every dimension (capability / totality / raw-write);
/// each must be rejected at load.  Positive controls prove the suite is not
/// vacuously rejecting everything — a sandbox that admits nothing is useless.
/// "No unknown holes" is unprovable, so this is how confidence is earned.
#[test]
#[expect(clippy::too_many_lines, reason = "inherited")]
fn admission_escape_suite_rejects_every_breakout() {
    // Parse + admit; ASSERT the probe parsed (a parse error would make the
    // admission check vacuous — a silent pass), then return the errors.
    fn adm(sel: &[&str], libs: &[&str], caps: &[&str], src: &str) -> Vec<String> {
        let p = parse_admit_libs(sel, libs, caps, src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "probe did not parse (vacuous):\n{src}\n{:?}",
            p.diagnostics.lines()
        );
        p.sandbox_admission_errors()
    }
    // A forbidden #cap native, declared in-source and NOT in an allowed library,
    // so the fine-grained capability gate applies to it.
    let secret = "capability danger\nfn secret() -> integer danger#read;\n#native\n";

    // ===== ESCAPES — admission MUST reject (≥1 error). =====
    let escapes: Vec<(&str, Vec<String>)> = vec![
        (
            "cap: direct ungranted call",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                &format!("{secret}fn evil() -> integer {{ secret() }}\n"),
            ),
        ),
        (
            "cap: indirect fn-ref (L4)",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                &format!("{secret}fn evil() -> integer {{ f = secret; f() }}\n"),
            ),
        ),
        (
            "cap: via sandboxed helper",
            adm(
                &["fn:evil", "fn:helper"],
                &["code"],
                &[],
                &format!(
                    "{secret}fn helper() -> integer {{ secret() }}\nfn evil() -> integer {{ helper() }}\n"
                ),
            ),
        ),
        (
            "totality: unbounded while",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "fn evil() { while true { } }\n",
            ),
        ),
        (
            "totality: self-recursion",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "fn evil() -> integer { evil() }\n",
            ),
        ),
        (
            "totality: mutual recursion",
            adm(
                &["fn:a", "fn:b"],
                &["code"],
                &[],
                "fn a() -> integer { b() }\nfn b() -> integer { a() }\n",
            ),
        ),
        (
            "totality: abort op assert",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "fn evil() { assert(false, \"x\") }\n",
            ),
        ),
        (
            "totality: abort op panic",
            adm(&["fn:evil"], &["code"], &[], "fn evil() { panic(\"x\") }\n"),
        ),
        (
            "totality: while non-constant step",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "fn evil(n: integer) { i = 0; j = 2; while i < n { i = i + j } }\n",
            ),
        ),
        (
            "totality: while conditional step",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "fn evil(n: integer, c: boolean) { i = 0; while i < n { if c { i = i + 1 } } }\n",
            ),
        ),
        (
            "cap: fn-ref in a collection (L4 — caught at fn-ref creation site)",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                &format!("{secret}fn evil() -> integer {{ v = [secret]; v[0]() }}\n"),
            ),
        ),
        (
            "cap: fn-ref returned then called (L4 — caught at fn-ref creation site)",
            adm(
                &["fn:evil", "fn:get"],
                &["code"],
                &[],
                &format!(
                    "{secret}fn get() -> fn() -> integer {{ secret }}\nfn evil() -> integer {{ g = get(); g() }}\n"
                ),
            ),
        ),
        (
            "cap: fn-ref in a struct field (L4 — caught at fn-ref creation site)",
            adm(
                &["fn:evil"],
                &["code", "prog"],
                &[],
                &format!(
                    "{secret}struct Holder {{ f: fn() -> integer }}\nfn evil() -> integer {{ h = Holder {{ f: secret }}; h.f() }}\n"
                ),
            ),
        ),
        (
            "raw-write: field",
            adm(
                &["fn:evil"],
                &["code", "prog"],
                &[],
                "struct Ent { hp: integer }\nfn evil(e: Ent) -> integer { e.hp = 0; e.hp }\n",
            ),
        ),
        (
            "raw-write: index",
            adm(
                &["fn:evil"],
                &["code", "prog"],
                &[],
                "fn evil(v: vector<integer>) -> integer { v[0] = 9; v[0] }\n",
            ),
        ),
        (
            // @PLN102 F6 — `r = &v` ALIASES the param `v` (proven: `r[0]=99` mutates the
            // caller's `v[0]`), so laundering a host-vector write through a `&`-bound
            // local must ALSO be rejected — the `Type::Vector => owned` gate must not
            // treat an alias of a parameter as script-owned.
            "raw-write: & alias launders param",
            adm(
                &["fn:evil"],
                &["code", "prog"],
                &[],
                "fn evil(v: vector<integer>) -> integer { r = &v; r[0] = 9; r[0] }\n",
            ),
        ),
        (
            "raw-write: nested field",
            adm(
                &["fn:evil"],
                &["code", "prog"],
                &[],
                "struct In { hp: integer }\nstruct Ent { it: In }\nfn evil(e: Ent) -> integer { e.it.hp = 0; e.it.hp }\n",
            ),
        ),
        (
            // @PLN86 D-cap-2 — a lambda BODY reaching an ungranted host cap. The
            // admission walk now DESCENDS into the lambda def (it is marked
            // sandboxed under the enclosing profile), so the reach is checked and
            // rejected — instead of the lambda escaping, OR being wholesale-rejected
            // as an untagged leaf without naming the real reach.
            "cap: lambda body reaches ungranted host (D-cap-2)",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                &format!(
                    "{secret}fn evil() -> integer {{ v = [1].map(|y| {{ secret() }}); v[0] }}\n"
                ),
            ),
        ),
        // loft#1930 — `owned(e)` is PROVENANCE: a view of a parameter is the caller's data
        // whatever its type.  The types here are the program's own and `prog` is not an
        // allowed library, which is the case the type rule read as script-owned.
        (
            "raw-write: a loop variable over a parameter's collection (loft#1930)",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "struct Ent { hp: integer }\nstruct W { ents: vector<Ent> }\nfn evil(w: W) { for e in w.ents { e.hp = 0 } }\n",
            ),
        ),
        (
            "raw-write: a parameter's element bound to a local (loft#1930)",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "struct Ent { hp: integer }\nstruct W { ents: vector<Ent> }\nfn evil(w: W) -> integer { x = w.ents[1]; x.hp = 7; x.hp }\n",
            ),
        ),
        (
            "raw-write: a `&` to a parameter's element (loft#1930)",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "struct Ent { hp: integer }\nstruct W { ents: vector<Ent> }\nfn evil(w: W) { r = &w.ents[0]; r.hp = 5 }\n",
            ),
        ),
        (
            "append: a vector parameter (loft#1930)",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "fn evil(v: vector<integer>) { v += [9] }\n",
            ),
        ),
        (
            "append: a `&` alias of a parameter's field (loft#1930)",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "struct W { xs: vector<integer> }\nfn evil(w: W) { q = &w.xs; q += [8] }\n",
            ),
        ),
        // loft#1929 — a whole-value read reads every field: rendering the value, as text
        // or JSON, and a `match` binding a `#read` field are reads of it.
        (
            "read: a `#read` field rendered with its struct (loft#1929)",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "capability secret\nstruct P { hidden: text secret#read, name: text }\nfn evil(p: P) -> text { \"{p}\" }\n",
            ),
        ),
        (
            "read: a `#read` field rendered as JSON (loft#1929)",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "capability secret\nstruct P { hidden: text secret#read, name: text }\nfn evil(p: P) -> text { \"{p:j}\" }\n",
            ),
        ),
        (
            "read: a `#read` field bound by a match (loft#1929)",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "capability secret\nstruct P { hidden: text secret#read, name: text }\nfn evil(p: P) -> text { match p { P { hidden } => hidden } }\n",
            ),
        ),
        (
            "read: a copy of a parameter is still the host's (loft#1929)",
            adm(
                &["fn:evil"],
                &["code"],
                &[],
                "capability secret\nstruct P { hidden: text secret#read, name: text }\nfn evil(p: P) -> text { c = P { hidden: \"x\", name: \"\" }; c = p; c.hidden }\n",
            ),
        ),
    ];
    for (name, e) in &escapes {
        assert!(!e.is_empty(), "ESCAPE NOT REJECTED — {name}");
    }

    // ===== CONTROLS — admission MUST admit (no errors). =====
    let controls: Vec<(&str, Vec<String>)> = vec![
        (
            "bounded for + arithmetic",
            adm(
                &["fn:ok"],
                &["code"],
                &[],
                "fn ok() -> integer { s = 0; for i in 0..10 { s += i } s }\n",
            ),
        ),
        (
            "bounded while (constant variant)",
            adm(
                &["fn:ok"],
                &["code"],
                &[],
                "fn ok() -> integer { i = 0; while i < 10 { i = i + 1 } i }\n",
            ),
        ),
        (
            "struct construction (not a write)",
            adm(
                &["fn:ok"],
                &["code", "prog"],
                &[],
                "struct Pt { x: integer }\nfn ok() -> Pt { Pt { x: 1 } }\n",
            ),
        ),
        (
            "local variable writes",
            adm(
                &["fn:ok"],
                &["code"],
                &[],
                "fn ok() -> integer { x = 5; x = x + 1; x }\n",
            ),
        ),
        (
            "granted capability",
            adm(
                &["fn:ok"],
                &["code"],
                &["danger#read"],
                &format!("{secret}fn ok() -> integer {{ secret() }}\n"),
            ),
        ),
        (
            // @PLN86 D-cap-2 — a lambda whose body touches ONLY script-owned data is
            // now usable in sandboxed code: the admission walk descends into the
            // lambda def and finds no host reach, so it admits (previously EVERY
            // lambda was rejected wholesale as an untagged `__lambda_N` leaf).
            "script-only lambda is usable (D-cap-2)",
            adm(
                &["fn:ok"],
                &["code"],
                &[],
                "fn ok() -> integer { s = 0; for x in [1,2,3].map(|y| { y * 2 }) { s += x } s }\n",
            ),
        ),
        (
            // @PLN86 D-cap-3 — a write to a SCRIPT-OWNED vector element is Cap-Own. A local
            // vector never aliases host state (every whole-value bind copies), so `v[i] = …`
            // on a local is admitted — the twin of the `raw-write: index` ESCAPE above, whose
            // `v[i] = …` is on a PARAMETER root (which DOES mutate the caller, so it rejects).
            "script-owned vector element write (D-cap-3)",
            adm(
                &["fn:ok"],
                &["code"],
                &[],
                "fn ok() -> integer { v = [1, 2, 3]; v[0] = 9; v[0] }\n",
            ),
        ),
        // loft#1929 — `Cap-Own`: a value the function BUILT reveals only what the script
        // put in it, so its `#read` field is free to read, field by field or whole.
        (
            "read: a `#read` field of a value built here (loft#1929)",
            adm(
                &["fn:ok"],
                &["code"],
                &[],
                "capability secret\nstruct P { hidden: text secret#read, name: text }\nfn ok() -> text { x = P { hidden: \"mine\", name: \"\" }; x.hidden }\n",
            ),
        ),
        (
            "read: a value built here rendered and matched (loft#1929)",
            adm(
                &["fn:ok"],
                &["code"],
                &[],
                "capability secret\nstruct P { hidden: text secret#read, name: text }\nfn ok() -> text { x = P { hidden: \"mine\", name: \"n\" }; \"{x} {match x { P { hidden } => hidden }}\" }\n",
            ),
        ),
        (
            // loft#1930 — a compiler operator with no capability gate is the language core:
            // a vector constant, `reverse()` and a fn-ref call were "library `files`"
            // (where the stdlib declares them) and needed every file operation allowed.
            "core: a vector constant, reverse() and a fn-ref under `code` only (loft#1930)",
            adm(
                &["fn:ok"],
                &["code"],
                &[],
                "NUMS = [3, 4, 5];\nfn ok() -> integer { s = 0; for n in NUMS { s += n; } v = [3, 1, 2]; v.reverse(); s + v[0] }\n",
            ),
        ),
        (
            "write: a local vector appended to (loft#1930's control)",
            adm(
                &["fn:ok"],
                &["code"],
                &[],
                "fn ok() -> integer { v = [1]; v += [2]; len(v) }\n",
            ),
        ),
    ];
    for (name, e) in &controls {
        assert!(e.is_empty(), "CLEAN SCRIPT REJECTED — {name}: {e:?}");
    }
}

/// loft#1930, `@FR-Cap-Write` — `r = append ⟹ m : collection`.  An `#append` link on a scalar
/// field is refused where it is declared, and a scalar `+=` is an UPDATE: it needs the
/// field's `#update` grant and no `#append` grant stands in for one.
#[test]
fn a_scalar_field_has_no_append_right() {
    let declared = parse_admit_libs(
        &["fn:f"],
        &["code"],
        &["bag#append"],
        "capability bag\nstruct M { sc: integer bag#append }\nfn f(m: M) { m.sc += 1 }\n",
    );
    assert!(
        declared
            .diagnostics
            .lines()
            .iter()
            .any(|l| l.contains("`bag#append` on `sc`: an append grows a collection")),
        "the scalar `#append` link is refused: {:?}",
        declared.diagnostics.lines()
    );
    let src = "capability bag\nstruct M { sc: integer bag#update }\nfn f(m: M) { m.sc += 1 }\n";
    let granted = parse_admit_libs(&["fn:f"], &["code"], &["bag#update"], src);
    assert!(
        granted.diagnostics.level() < crate::diagnostics::Level::Error
            && granted.sandbox_admission_errors().is_empty(),
        "a scalar `+=` under its update grant is admitted: {:?}",
        granted.sandbox_admission_errors()
    );
    let ungranted = parse_admit_libs(&["fn:f"], &["code"], &[], src);
    assert!(
        !ungranted.sandbox_admission_errors().is_empty(),
        "a scalar `+=` without its update grant is refused"
    );
}

/// @PLN86 P8.2 (F13) — the RED/GREEN ACCESS corpus: the committed battery over the
/// capability access model (function call gate, field read/update/append, parameter
/// `#default` lock, the `files` library split, undeclared links).  Every RED is
/// paired with a GREEN TWIN — the SAME code with the grant added must ADMIT — so a
/// rejection is proven to be the rule firing, not a parse error or an unrelated
/// reject (non-vacuity, the escape-suite discipline).  Plus standalone REDs
/// (read-only-by-default) and GREENs (construction is unrestricted, reads are free).
#[test]
#[expect(clippy::too_many_lines, reason = "inherited")]
fn access_corpus_red_green() {
    // parse + assert the probe parsed (else admission is vacuous) → errors.
    fn adm(sel: &[&str], libs: &[&str], caps: &[&str], src: &str) -> Vec<String> {
        let p = parse_admit_libs(sel, libs, caps, src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "probe did not parse (vacuous):\n{src}\n{:?}",
            p.diagnostics.lines()
        );
        p.sandbox_admission_errors()
    }
    // A RED (ungranted) rejects naming `token`; its GREEN twin (granted) does NOT —
    // proving the rejection is the access rule, not an incidental failure.
    let twin = |name: &str,
                libs: &[&str],
                token: &str,
                ungranted: &[&str],
                granted: &[&str],
                src: &str| {
        let red = adm(&["fn:f"], libs, ungranted, src);
        assert!(
            red.iter().any(|e| e.contains(token)),
            "{name}: RED must be rejected naming `{token}`: {red:?}"
        );
        let green = adm(&["fn:f"], libs, granted, src);
        assert!(
            !green.iter().any(|e| e.contains(token)),
            "{name}: GREEN twin (granted) must admit — non-vacuity: {green:?}"
        );
    };

    // ── call gate: an `fs#update` function under an `fs#read`-only grant ──
    twin(
        "call gate fs#update under fs#read",
        &["code"],
        "fs#update",
        &["fs#read"],
        &["fs#read", "fs#update"],
        "fn host_write() -> integer fs#update;\n#native\n\
         fn f() -> integer { host_write() }\n",
    );
    // ── field READ of a private (`#read`-linked) field ──
    twin(
        "field read (private)",
        &["code"],
        "secret#read",
        &[],
        &["secret#read"],
        "capability secret\nstruct P { hidden: text secret#read }\n\
         fn f(p: P) -> text { p.hidden }\n",
    );
    // ── field UPDATE of an `#update`-linked field ──
    twin(
        "field update",
        &["code"],
        "stats#update",
        &[],
        &["stats#update"],
        "capability stats\nstruct M { hp: integer stats#update }\n\
         fn f(m: M) { m.hp = 0 }\n",
    );
    // ── field APPEND to a `#append`-linked collection field ──
    twin(
        "field append",
        &["code"],
        "bag#append",
        &[],
        &["bag#append"],
        "capability bag\nstruct I { items: vector<integer> bag#append }\n\
         fn f(i: I, x: integer) { i.items += [x] }\n",
    );
    // ── parameter `#default` lock: overriding a pinned argument ──
    twin(
        "param #default lock override",
        &["code"],
        "spawn.count#default",
        &["world#append"],
        &["world#append", "spawn.count#default"],
        "capability world\ncapability spawn.count\n\
         fn spawn(kind: text, count: integer = 1 spawn.count#default) -> integer world#append;\n#native\n\
         fn f() -> integer { spawn(\"g\", 5) }\n",
    );

    // ── @PLN86 D-cap-2: a lambda BODY reaching a host cap — gated by descending
    //    into the lambda def (marked sandboxed under the enclosing profile) ──
    twin(
        "lambda body reaches host cap (D-cap-2)",
        &["code"],
        "danger#read",
        &[],
        &["danger#read"],
        "capability danger\nfn secret() -> integer danger#read;\n#native\n\
         fn f() -> integer { v = [1].map(|y| { secret() }); v[0] }\n",
    );

    // ── undeclared capability link (typo) — RED + corrected-spelling GREEN twin ──
    let typo = adm(
        &["fn:f"],
        &["code"],
        &["helth#update"],
        "capability health\nstruct M { hp: integer helth#update }\nfn f(m: M) { m.hp = 0 }\n",
    );
    assert!(
        typo.iter()
            .any(|e| e.contains("undeclared capability `helth`")),
        "undeclared link must reject naming the group: {typo:?}"
    );
    let fixed = adm(
        &["fn:f"],
        &["code"],
        &["health#update"],
        "capability health\nstruct M { hp: integer health#update }\nfn f(m: M) { m.hp = 0 }\n",
    );
    assert!(
        !fixed.iter().any(|e| e.contains("undeclared")),
        "the corrected spelling must admit (non-vacuity): {fixed:?}"
    );

    // ===== standalone REDs — read-only-by-default (no grant admits them) =====
    // an UNLINKED host field stays read-only even when a capability is granted.
    let unlinked = adm(
        &["fn:f"],
        &["code"],
        &["stats#update"],
        "struct M { hp: integer }\nfn f(m: M) { m.hp = 0 }\n",
    );
    assert!(
        !unlinked.is_empty(),
        "an unlinked host field must stay read-only: {unlinked:?}"
    );
    // append-only: `=` (update) on a field with `#append` but NO `#update` is rejected
    // even with append granted — append-only is exactly this.
    let append_only_update = adm(
        &["fn:f"],
        &["code"],
        &["bag#append"],
        "capability bag\nstruct I { items: vector<integer> bag#append }\n\
         fn f(i: I) { i.items = [1] }\n",
    );
    assert!(
        !append_only_update.is_empty(),
        "update via `=` on an append-only field must be rejected: {append_only_update:?}"
    );

    // ===== standalone GREENs — the model must not over-reject =====
    // CONSTRUCTION is unrestricted (the position-1 decision — no construction gate).
    let construct = adm(
        &["fn:f"],
        &["code", "prog"],
        &[],
        "struct Pt { x: integer }\nfn f() -> Pt { Pt { x: 1 } }\n",
    );
    assert!(
        construct.is_empty(),
        "construction must admit (no construction gate): {construct:?}"
    );
    // reading an UNLINKED field is free (read is default-allow).
    let free_read = adm(
        &["fn:f"],
        &["code"],
        &[],
        "struct P { name: text }\nfn f(p: P) -> text { p.name }\n",
    );
    assert!(
        free_read.is_empty(),
        "reading an unlinked field is free: {free_read:?}"
    );
    // the REAL stdlib `files` split: `mtime` (fs#read) admits under an `fs#read` grant.
    let real_read = adm(
        &["fn:f"],
        &["code"],
        &["fs#read"],
        "fn f() -> integer { mtime(\"x\") }\n",
    );
    assert!(
        real_read.is_empty(),
        "real stdlib mtime (fs#read) must admit under fs#read: {real_read:?}"
    );
}

/// @PLN86 2.4 — no-raw-write rejects a field/index assignment to heap data,
/// while struct construction + local-variable writes stay clean.
#[test]
fn no_raw_write_rejects_field_index_admits_construction() {
    // field write → rejected, with an actionable message
    let p = parse_admit_libs(
        &["fn:scripted"],
        &["code", "prog"],
        &[],
        "struct Ent { health: integer }\n\
         fn scripted(e: Ent) -> integer { e.health = 0; e.health }\n",
    );
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    assert_eq!(
        p.sandbox_raw_writes().len(),
        1,
        "a field write must be flagged, got {:?}",
        p.sandbox_raw_writes()
    );
    assert!(
        p.sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("raw write") && e.contains("fix")),
        "{:?}",
        p.sandbox_admission_errors()
    );

    // index write → rejected
    let p2 = parse_admit_libs(
        &["fn:scripted"],
        &["code", "prog"],
        &[],
        "fn scripted(v: vector<integer>) -> integer { v[0] = 9; v[0] }\n",
    );
    assert!(
        !p2.sandbox_raw_writes().is_empty(),
        "an index write must be flagged, got {:?}",
        p2.sandbox_raw_writes()
    );

    // struct construction + local writes → admitted (no raw write)
    let p3 = parse_admit_libs(
        &["fn:scripted"],
        &["code", "prog"],
        &[],
        "struct Ent { health: integer }\n\
         fn scripted() -> integer { s = 0; for i in 0..3 { s += i } e = Ent { health: s }; e.health }\n",
    );
    assert!(
        p3.sandbox_raw_writes().is_empty(),
        "construction + local writes must be clean, got {:?}",
        p3.sandbox_raw_writes()
    );
}

/// @PLN86 2.4 (ownership-aware) — a mod may MUTATE the data it owns (a local of
/// a script-defined struct — the dogfood `e.alive = false` pattern), but NOT
/// host data (a parameter — the type also catches aliasing).
#[test]
fn no_raw_write_admits_script_owned_struct_mutation() {
    // local script-owned struct mutation → admitted (no violation at all)
    let p = parse_admit_libs(
        &["fn:scripted"],
        &["code"],
        &[],
        "struct Mob { hp: integer }\n\
         fn scripted() -> integer { m = Mob { hp: 5 }; m.hp = 0; m.hp }\n",
    );
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    assert!(
        p.sandbox_raw_writes().is_empty(),
        "mutating a script-owned struct must be allowed, got {:?}",
        p.sandbox_raw_writes()
    );
    assert!(
        p.sandbox_admission_errors().is_empty(),
        "a self-owned mutation script must fully admit, got {:?}",
        p.sandbox_admission_errors()
    );

    // the SAME write to a PARAMETER (host data) is still rejected
    let p2 = parse_admit_libs(
        &["fn:scripted"],
        &["code"],
        &[],
        "struct Mob { hp: integer }\n\
         fn scripted(m: Mob) -> integer { m.hp = 0; m.hp }\n",
    );
    assert!(
        !p2.sandbox_raw_writes().is_empty(),
        "a write to a parameter (host data) must be rejected, got {:?}",
        p2.sandbox_raw_writes()
    );
}

/// @PLN86 P6.4 (F4) — a sandboxed read of a `#read`-linked HOST field needs the
/// grant; an unlinked field is freely readable (read default-allow).
#[test]
fn field_read_gates_private_field_admits_unlinked() {
    let src = "capability secret\n\
               struct Player { name: text, hidden: text secret#read }\n\
               fn peek(p: Player) -> text { p.hidden }\n\
               fn nameof(p: Player) -> text { p.name }\n";
    // ungranted: reading `hidden` (secret#read) is rejected; `name` (unlinked) is fine.
    let p = parse_admit_libs(&["fn:peek", "fn:nameof"], &["code"], &[], src);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    let errs = p.sandbox_admission_errors();
    assert!(
        errs.iter().any(|e| e.contains("secret#read")),
        "a private field read must be rejected: {errs:?}"
    );
    // granted: the same read admits clean.
    let p2 = parse_admit_libs(&["fn:peek", "fn:nameof"], &["code"], &["secret#read"], src);
    assert!(
        !p2.sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("secret#read")),
        "granted secret#read must admit the read: {:?}",
        p2.sandbox_admission_errors()
    );
}

/// @PLN86 P6.4 (F5) — a write to an `#update`-linked host field admits iff the
/// token is granted; a field with NO update link stays read-only (the coarse 2.4
/// reject), generalising the all-or-nothing no-raw-write.
#[test]
fn field_update_gates_writable_fields_unlinked_read_only() {
    let src = "capability stats\n\
               struct Mob { hp: integer stats#update, id: integer }\n\
               fn hurt(m: Mob) -> integer { m.hp = 0; m.hp }\n";
    // ungranted: writing `hp` (stats#update) is rejected.
    let p = parse_admit_libs(&["fn:hurt"], &["code"], &[], src);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    assert!(
        p.sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("stats#update")),
        "an ungranted field write must be rejected: {:?}",
        p.sandbox_admission_errors()
    );
    // granted: the same write admits clean.
    let p2 = parse_admit_libs(&["fn:hurt"], &["code"], &["stats#update"], src);
    assert!(
        p2.sandbox_admission_errors().is_empty(),
        "granted stats#update must admit the write: {:?}",
        p2.sandbox_admission_errors()
    );
    // a field with NO update link stays read-only even when a cap is granted.
    let src2 = "struct Mob { hp: integer }\n\
                fn hurt(m: Mob) -> integer { m.hp = 0; m.hp }\n";
    let p3 = parse_admit_libs(&["fn:hurt"], &["code"], &["stats#update"], src2);
    assert!(
        !p3.sandbox_admission_errors().is_empty(),
        "an unlinked host field must stay read-only: {:?}",
        p3.sandbox_admission_errors()
    );
}

/// @PLN86 P6.4 (F6) — `e.f += x` growing an `#append`-linked collection field admits
/// iff the token is granted; an append to a field with no append link falls to the
/// `#update`/coarse path.  This is what makes append-only expressible.
#[test]
fn field_append_gates_collection_grow() {
    let src = "capability bag\n\
               struct Inv { items: vector<integer> bag#append }\n\
               fn add(i: Inv, x: integer) { i.items += [x] }\n";
    // ungranted: appending to `items` (bag#append) is rejected.
    let p = parse_admit_libs(&["fn:add"], &["code"], &[], src);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    assert!(
        p.sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("bag#append")),
        "an ungranted append must be rejected: {:?}",
        p.sandbox_admission_errors()
    );
    // granted: the same append admits clean.
    let p2 = parse_admit_libs(&["fn:add"], &["code"], &["bag#append"], src);
    assert!(
        p2.sandbox_admission_errors().is_empty(),
        "granted bag#append must admit the grow: {:?}",
        p2.sandbox_admission_errors()
    );
}

/// @PLN86 §7.2 (F7) — a parameter the host pinned with `…#default` is forced to its
/// default unless the modder holds the lock: omitting the argument (`spawn("g")`) is
/// free, but OVERRIDING it (`spawn("g", 5)`) needs `spawn.count#default` granted.
#[test]
fn param_default_lock_gates_override() {
    // `world#append` is the call gate (needed to call spawn at all); `count` is then
    // pinned to its default `1` by `spawn.count#default`.
    let src = "capability world\n\
               capability spawn.count\n\
               fn spawn(kind: text, count: integer = 1 spawn.count#default) \
                   -> integer world#append;\n#native\n\
               fn bare() -> integer { spawn(\"goblin\") }\n\
               fn override_count() -> integer { spawn(\"goblin\", 5) }\n";
    // call-gate granted, lock NOT granted: the bare call admits, the override is rejected.
    let p = parse_admit_libs(
        &["fn:bare", "fn:override_count"],
        &["code"],
        &["world#append"],
        src,
    );
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    let errs = p.sandbox_admission_errors();
    assert!(
        errs.iter().any(|e| e.contains("spawn.count#default")),
        "overriding a pinned parameter must be rejected: {errs:?}"
    );
    // the bare call (uses the default) must NOT be flagged — only the override is.
    let bare = p.data.def_nr("n_bare");
    assert!(
        !p.sandbox_param_overrides.contains_key(&bare),
        "a defaulted parameter is not an override: {:?}",
        p.sandbox_param_overrides.get(&bare)
    );
    // lock granted: the override admits clean.
    let p2 = parse_admit_libs(
        &["fn:bare", "fn:override_count"],
        &["code"],
        &["world#append", "spawn.count#default"],
        src,
    );
    assert!(
        !p2.sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("spawn.count#default")),
        "granted spawn.count#default must admit the override: {:?}",
        p2.sandbox_admission_errors()
    );
}

/// @PLN86 P6.8 (F8) — a `group#right` link in the main program whose group was never
/// declared as a `capability` is a clean LOAD error (today it would silently deny, the
/// group never matching a grant); a correctly-declared link is not flagged.
#[test]
fn undeclared_capability_link_is_a_load_error() {
    // a field link to `helth` — a typo for the `health` capability actually declared.
    let typo = "capability health\n\
                struct Mob { hp: integer helth#update }\n\
                fn hurt(m: Mob) { m.hp = 0 }\n";
    let p = parse_admit_libs(&["fn:hurt"], &["code"], &["helth#update"], typo);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    let errs = p.sandbox_admission_errors();
    assert!(
        errs.iter()
            .any(|e| e.contains("undeclared capability `helth`")),
        "an undeclared capability group must be a load error: {errs:?}"
    );
    // the correctly-spelled link resolves against its declaration — not flagged.
    let ok = "capability health\n\
              struct Mob { hp: integer health#update }\n\
              fn hurt(m: Mob) { m.hp = 0 }\n";
    let p2 = parse_admit_libs(&["fn:hurt"], &["code"], &["health#update"], ok);
    assert!(
        !p2.sandbox_admission_errors()
            .iter()
            .any(|e| e.contains("undeclared capability")),
        "a declared link must not be flagged: {:?}",
        p2.sandbox_admission_errors()
    );
}

/// @PLN86 L4 — a fn-ref to a forbidden capability hidden where `referenced_defs`
/// can't see it (a COLLECTION element — neither a call nor an assignment) is
/// still capability-checked.  `mtime` (tagged `fs.read`) as a VALUE inside
/// `[mtime]` must be rejected exactly as a direct `mtime(..)` call would —
/// because it is recorded at the fn-ref CREATION site, not by an IR walk that
/// only catches call-args + assignments.
#[test]
fn l4_fn_ref_in_collection_cannot_escape_capability() {
    let p = parse_admit_libs(
        &["fn:scripted"],
        &["code"],
        &["env"], // fs.read NOT granted
        "fn scripted() -> integer { handlers = [mtime]; len(handlers) }\n",
    );
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    let scripted = p.data.def_nr("n_scripted");
    let mtime = p.data.def_nr("n_mtime");
    // recorded at creation — the L4 mechanism
    assert!(
        p.sandbox_fn_refs
            .get(&scripted)
            .is_some_and(|s| s.contains(&mtime)),
        "the fn-ref to mtime must be recorded for L4, got {:?}",
        p.sandbox_fn_refs.get(&scripted)
    );
    // and admission rejects it naming the capability — no escape
    let v = p.sandbox_admit();
    assert!(
        v.iter().any(|x| viol_symbol(x) == mtime),
        "a collection-hidden fn-ref to mtime must be capability-checked (L4), got {v:?}"
    );
}

/// @PLN86 3.1 — does the sandboxed program have an unbounded-`while` violation?
fn has_unbounded_while(p: &Parser) -> bool {
    p.sandbox_totality()
        .iter()
        .any(|v| matches!(v, TotalityViolation::UnboundedLoop { .. }))
}

/// @PLN86 3.1 — a `while` carrying a compiler-checked DECREASING VARIANT is
/// admitted: an int counter against a stable bound, stepped by a constant every
/// iteration (counting up, counting down, or guarded by `&& i < N`).
#[test]
fn bounded_while_is_admitted() {
    for (label, src) in [
        (
            "count up to a param bound",
            "fn scripted(n: integer) -> integer { i = 0; s = 0; \
             while i < n { s = s + i; i = i + 1; } s }\n",
        ),
        (
            "count down to zero",
            "fn scripted() -> integer { j = 10; s = 0; \
             while j > 0 { s = s + j; j = j - 1; } s }\n",
        ),
        (
            "guard counter in a conjunction",
            "fn scripted(flag: boolean) -> integer { g = 0; s = 0; \
             while flag && (g < 2000) { s = s + g; g = g + 1; } s }\n",
        ),
    ] {
        let p = parse_admit_libs(&["fn:scripted"], &["code"], &[], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "{label}: parse errors {:?}",
            p.diagnostics.lines()
        );
        assert!(
            !has_unbounded_while(&p),
            "{label}: a bounded while must be admitted, got {:?}",
            p.sandbox_totality()
        );
    }
}

/// @PLN86 3.1 — SOUNDNESS: a `while` whose termination cannot be PROVEN is
/// rejected.  Each of these is either a genuine non-terminator or one the
/// conservative recognizer must refuse (unsoundness here would let a script
/// hang the host).
#[test]
fn unprovable_while_is_rejected() {
    for (label, src) in [
        (
            "flag loop (no variant)",
            "fn scripted(running: boolean) -> integer { s = 0; \
             while running { s = s + 1; } s }\n",
        ),
        (
            "no step (counter never moves)",
            "fn scripted(n: integer) -> integer { i = 0; s = 0; \
             while i < n { s = s + i; } s }\n",
        ),
        (
            "conditional step (may not run every iteration)",
            "fn scripted(n: integer) -> integer { i = 0; \
             while i < n { if i > 5 { i = i + 1; } } i }\n",
        ),
        (
            "cancelling steps (net non-monotonic)",
            "fn scripted(n: integer) -> integer { i = 0; \
             while i < n { i = i + 1; i = i - 1; } i }\n",
        ),
        (
            "non-constant step (cannot prove > 0)",
            "fn scripted(n: integer, k: integer) -> integer { i = 0; \
             while i < n { i = i + k; } i }\n",
        ),
        (
            "moving bound (races away)",
            "fn scripted(n: integer) -> integer { i = 0; m = n; \
             while i < m { i = i + 1; m = m + 1; } i }\n",
        ),
    ] {
        let p = parse_admit_libs(&["fn:scripted"], &["code"], &[], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "{label}: parse errors {:?}",
            p.diagnostics.lines()
        );
        assert!(
            has_unbounded_while(&p),
            "{label}: an unprovable while must be rejected"
        );
    }
}

/// `@FR-Cap-Call` — an UNGATED stdlib function is admitted under a profile that allows no
/// library at all, because it computes only on its arguments; a host-touching one names its
/// gate, and a grant of that gate admits it.
#[test]
fn an_ungated_stdlib_function_is_admitted_with_no_library_allowed() {
    let src = "fn pure() -> integer { v = [3, 1, 2]; v.sort(); len(\"abc\") * 2 + v[0] }\n\
               fn prints() { println(\"x\"); }\n\
               fn quits() { exit(3); }\n\
               fn clock() -> integer { now() }\n";
    let p = parse_admit(&["fn:pure", "fn:prints", "fn:quits", "fn:clock"], &[], src);
    let v = p.sandbox_admit();
    let from = |n: &str| p.data.def_nr(n);
    assert!(
        !v.iter().any(|x| x.from() == from("n_pure")),
        "a pure stdlib computation must be admitted with no grant, got {v:?}"
    );
    for (f, sym, gate) in [
        ("n_prints", "n_println", "io#append"),
        ("n_quits", "n_exit", "process#update"),
        ("n_clock", "n_now", "time#read"),
    ] {
        assert!(
            v.contains(&CapViolation::UngrantedCap {
                from: from(f),
                symbol: from(sym),
                group: gate.to_string(),
            }),
            "{f} must be refused naming {gate}, got {v:?}"
        );
    }
    let granted = parse_admit(
        &["fn:prints"],
        &["io#append"],
        "fn prints() { println(\"x\"); }\n",
    );
    let gv = granted.sandbox_admit();
    assert!(
        gv.is_empty(),
        "println under io#append must be admitted, got {gv:?}"
    );
}

/// The reviewed list of UNGATED stdlib functions — each one is admitted in every sandbox, so
/// each must compute only on its arguments.  A function added to `default/` without a gate
/// fails here until it is either gated (it reaches the host: a file, the environment, the
/// clock, the output stream, the process, a thread) or confirmed safe and added to
/// `index/stdlib_ungated.txt` (`LOFT_BLESS=1` rewrites the list).
#[test]
fn the_ungated_stdlib_surface_is_the_reviewed_list() {
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    let mut names: Vec<String> = (0..p.data.definitions())
        .filter(|&d| {
            let def = p.data.def(d);
            matches!(def.def_type(), crate::data::DefType::Function)
                && def.source == crate::data::STD_SOURCE
                && def.synthetic().is_none()
                && def.cap().is_empty()
        })
        .map(|d| p.data.def(d).name().to_string())
        .collect();
    names.sort();
    names.dedup();
    let list = format!("{}\n", names.join("\n"));
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/index/stdlib_ungated.txt");
    if std::env::var_os("LOFT_BLESS").is_some() {
        crate::file_access::write(path, &list).unwrap();
        return;
    }
    let want = crate::file_access::read_to_string(path).unwrap_or_default();
    let want: std::collections::BTreeSet<&str> = want.lines().collect();
    let have: std::collections::BTreeSet<&str> = names.iter().map(String::as_str).collect();
    let added: Vec<_> = have.difference(&want).collect();
    let gone: Vec<_> = want.difference(&have).collect();
    assert!(
        added.is_empty() && gone.is_empty(),
        "the ungated stdlib surface moved — every ungated stdlib function is admitted in every \
         sandbox.\n  new, ungated: {added:?} (gate each that reaches the host)\n  no longer \
         ungated or gone: {gone:?}\n  then re-pin: LOFT_BLESS=1 cargo test --lib \
         the_ungated_stdlib_surface_is_the_reviewed_list"
    );
}
