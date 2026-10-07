// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN86 — the sandbox, end-to-end through the `loft` binary: a program's
//! `[sandbox]` policy loads from `loft.toml`, a violating sandboxed program is
//! rejected at load with an actionable error, a clean one is admitted and runs, and
//! a clean sandboxed program also runs on `--native` (admission is backend-agnostic —
//! the forced interpret-only was dropped, an admitted script is fault-free on both).

use loft::file_access as fa;
use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Write `prog.loft` + `loft.toml` into a fresh temp dir, run the binary (the
/// stdlib is found relative to the workspace root), return `(success, stderr)`.
fn run(name: &str, prog: &str, toml: &str, native: bool) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("loft_sbcli_{}_{name}", std::process::id()));
    let _ = fa::create_dir_all(&dir);
    fa::write(dir.join("prog.loft"), prog).unwrap();
    fa::write(dir.join("loft.toml"), toml).unwrap();
    let mut cmd = Command::new(loft_bin());
    if native {
        cmd.arg("--native");
    }
    cmd.arg("--timeout")
        .arg("60")
        .arg(dir.join("prog.loft"))
        .current_dir(workspace_root());
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = fa::remove_dir_all(&dir);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const POLICY: &str = "[sandbox]\nmod = [\"fn:scripted\"]\n[profile.mod]\nallow_libs = [\"code\"]\n";

/// A sandboxed def reaching `mtime` (fs#read) under a policy that grants only
/// `code` is rejected, with the actionable error naming the symbol + group + fix.
#[test]
fn sandboxed_capability_violation_is_rejected() {
    let prog = "fn scripted() -> integer { mtime(\"x\") }\nfn main() -> integer { scripted() }\n";
    let (ok, err) = run("viol", prog, POLICY, false);
    assert!(!ok, "a violating sandboxed program must be rejected");
    assert!(
        err.contains("admission violation")
            && err.contains("mtime")
            && err.contains("fs#read")
            && err.contains("fix:"),
        "stderr: {err}"
    );
}

/// Sandboxed recursion is rejected at admission — and the fix-it hands the
/// user the supported path (Goal F): the stdlib's recursion-free traversal
/// (`Walkable` + `tree_walk`), since a tree walk is what most rejected
/// recursion is doing.
#[test]
fn sandboxed_recursion_rejection_names_tree_walk() {
    let prog = "struct Tn { t_v: integer, t_kids: vector<Tn> }\n\
                fn scripted(sn: Tn) -> integer {\n\
                  st = sn.t_v;\n\
                  for sk in sn.t_kids { st += scripted(sk); }\n\
                  return st;\n\
                }\n\
                fn main() -> integer { scripted(Tn { t_v: 5, t_kids: [] }) }\n";
    let (ok, err) = run("recur", prog, POLICY, false);
    assert!(!ok, "sandboxed recursion must be rejected");
    assert!(
        err.contains("recursion is not permitted")
            && err.contains("fix:")
            && err.contains("tree_walk")
            && err.contains("Walkable"),
        "the rejection must name the recursion-free traversal; stderr: {err}"
    );
}

/// A clean sandboxed program (only `code`-library arithmetic) is admitted and
/// runs (force-interpreted).
#[test]
fn sandboxed_clean_program_is_admitted() {
    let prog = "fn scripted() -> integer { 21 + 21 }\nfn main() -> integer { scripted() }\n";
    let (ok, err) = run("clean", prog, POLICY, false);
    assert!(
        ok,
        "a clean sandboxed program must be admitted + run; stderr: {err}"
    );
}

/// A clean sandboxed program runs on `--native` too: admission is backend-agnostic
/// (an admitted script is total + fault-free on both backends), so the forced
/// interpret-only was dropped and `--native` is no longer refused.
#[test]
fn sandboxed_clean_program_runs_on_native() {
    let prog = "fn scripted() -> integer { 21 + 21 }\nfn main() -> integer { scripted() }\n";
    let (ok, err) = run("native", prog, POLICY, true);
    assert!(
        ok,
        "a clean sandboxed program must be admitted + run on --native; stderr: {err}"
    );
    assert!(
        !err.contains("interpret-only"),
        "--native must no longer be refused for sandboxed code: {err}"
    );
}

/// @PLN86 — the program warm-cache must NOT bypass admission.  A whole-program
/// warm load restores the IR without re-parsing, so `def_sandbox` would never form
/// and admission would be skipped; and the cache is keyed by program CONTENT, not
/// policy.  So a program admitted under a permissive policy,
/// then run UNCHANGED under a tightened policy, must still be re-admitted — and
/// rejected.  Runs with the program cache on (`LOFT_NO_CACHE` removed) and an
/// isolated `LOFT_HOME`, so run 1 writes a warm cache that run 2 must ignore.
#[test]
fn warm_program_cache_does_not_bypass_admission() {
    let dir = std::env::temp_dir().join(format!("loft_sbcache_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    let home = dir.join("home");
    fa::create_dir_all(&home).unwrap();
    let prog = dir.join("prog.loft");
    fa::write(
        &prog,
        "fn scripted() -> integer { mtime(\"Cargo.toml\") }\n\
         fn main() -> integer { scripted() }\n",
    )
    .unwrap();

    let run = |toml: &str| -> (bool, String) {
        fa::write(dir.join("loft.toml"), toml).unwrap();
        let out = Command::new(loft_bin())
            .env_remove("LOFT_NO_CACHE")
            .env("LOFT_HOME", &home)
            .arg("--timeout")
            .arg("60")
            .arg(&prog)
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    // permissive: fs#read granted → admitted, runs, and WARMS the program cache.
    let permissive = "[sandbox]\nmod = [\"fn:scripted\"]\n\
                      [profile.mod]\nallow_libs = [\"code\"]\nallow = [\"fs#read\"]\n";
    let (ok1, err1) = run(permissive);
    assert!(ok1, "permissive run must be admitted and run: {err1}");

    // tightened (fs#read withdrawn): the SAME program must be rejected even though
    // a warm cache now exists — no bypass.
    let restrictive = "[sandbox]\nmod = [\"fn:scripted\"]\n\
                       [profile.mod]\nallow_libs = [\"code\"]\n";
    let (ok2, err2) = run(restrictive);
    let _ = fa::remove_dir_all(&dir);
    assert!(
        !ok2,
        "the tightened policy must reject on the warm run (admission not bypassed): {err2}"
    );
    assert!(
        err2.contains("admission violation") && err2.contains("fs#read"),
        "warm-run rejection must name the capability: {err2}"
    );
}

/// A raw field write to heap data in a sandboxed def is rejected (2.4) — mutation
/// must go through a `*.write` op; construction stays fine.
#[test]
fn sandboxed_raw_write_is_rejected() {
    let prog = "struct Ent { health: integer }\n\
                fn scripted(e: Ent) -> integer { e.health = 0; e.health }\n\
                fn main() -> integer { scripted(Ent { health: 5 }) }\n";
    let (ok, err) = run("rawwrite", prog, POLICY, false);
    assert!(!ok, "a raw write to host data must be rejected");
    assert!(
        err.contains("raw write") && err.contains("fix:"),
        "stderr: {err}"
    );
}

/// @PLN86 F12 — `loft sandbox-check <file>` reports the admission verdict and NEVER
/// executes: a clean program prints `Admitted` (exit 0) without running its
/// side-effecting `main`; a violating one prints `Rejected` + diagnostics (exit 1).
#[test]
fn sandbox_check_reports_verdict_without_executing() {
    // Run `sandbox-check <file>` in a temp dir with a loft.toml; capture BOTH streams
    // (Admitted → stdout, Rejected → stderr) and the exit status.
    fn check(name: &str, prog: &str, toml: &str) -> (bool, String) {
        let dir = std::env::temp_dir().join(format!("loft_sccli_{}_{name}", std::process::id()));
        let _ = fa::create_dir_all(&dir);
        fa::write(dir.join("prog.loft"), prog).unwrap();
        fa::write(dir.join("loft.toml"), toml).unwrap();
        let out = Command::new(loft_bin())
            .arg("sandbox-check")
            .arg("--timeout")
            .arg("60")
            .arg(dir.join("prog.loft"))
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let _ = fa::remove_dir_all(&dir);
        let mut both = String::from_utf8_lossy(&out.stdout).into_owned();
        both.push_str(&String::from_utf8_lossy(&out.stderr));
        (out.status.success(), both)
    }

    // The side-effecting main must NOT run in either verdict.
    let sentinel = "EXECUTED-SHOULD-NOT-PRINT";

    // clean program → Admitted (exit 0), complexity reported, main NOT run.
    let clean = format!(
        "fn scripted() -> integer {{ s = 0; for i in 0..10 {{ s += i }} s }}\n\
         fn main() {{ print(\"{sentinel}\") }}\n"
    );
    let (ok, out) = check("clean", &clean, POLICY);
    assert!(ok, "a clean program must be Admitted (exit 0): {out}");
    assert!(out.contains("Admitted"), "must print Admitted: {out}");
    assert!(
        !out.contains(sentinel),
        "sandbox-check must NOT execute the program: {out}"
    );

    // violating program → Rejected (exit 1), diagnostics, main NOT run.
    let bad = format!(
        "fn scripted() -> integer {{ mtime(\"x\") }}\n\
         fn main() {{ print(\"{sentinel}\") }}\n"
    );
    let (ok2, out2) = check("bad", &bad, POLICY);
    assert!(
        !ok2,
        "a violating program must be Rejected (exit 1): {out2}"
    );
    assert!(
        out2.contains("Rejected") && out2.contains("fs#read") && out2.contains("fix:"),
        "must print Rejected + diagnostics: {out2}"
    );
    assert!(
        !out2.contains(sentinel),
        "a rejected sandbox-check must NOT execute the program: {out2}"
    );
}

/// @PLN24 — a `#c` binding is gated by `native_ffi`, never by a `#cap` grant.
///
/// The plan left this as an open design question ("does a `#c` symbol need an
/// effect declaration to be admissible?") and the answer came from measuring the
/// tree rather than reading it: a sandboxed script reaching a `#c` binding
/// tagged `db#read`, under a profile that granted `db#read` and left `native_ffi`
/// at its default false, was **admitted and ran the C**.
///
/// The reason is worth keeping beside the test.  Both the FFI gate and
/// `reachable_ffi_bridges` key on `def.native()`, which arc A leaves EMPTY on a
/// `#c` definition on purpose — so the Rust dispatch path cannot claim one — and
/// a `#c` binding therefore fell through to the capability check.  A capability
/// grant says what DATA a script may touch; it cannot say "and arbitrary machine
/// code may run in this process".  That is the line `native_ffi` already draws
/// for a Rust cdylib bridge, and a `#c` call is the stronger case: there is no
/// marshalling layer between the script and the library at all.
///
/// Three cells, because the fix has to reject exactly one of them.
#[test]
fn a_c_binding_is_gated_by_native_ffi_not_by_a_capability_grant() {
    let dir = std::env::temp_dir().join(format!("loft_sbcli_cbind_{}", std::process::id()));
    let pkg = dir.join("lib").join("cbind").join("src");
    fa::create_dir_all(&pkg).unwrap();
    fa::write(
        dir.join("lib").join("cbind").join("loft.toml"),
        "[library]\nname = \"cbind\"\nversion = \"0.0.1\"\n",
    )
    .unwrap();
    // libc, so the binding needs no `[c] libs` entry and no library on disk:
    // admission runs at LOAD, and what is under test is the verdict.
    fa::write(
        pkg.join("cbind.loft"),
        "pub fn c_len(s: text) -> integer db#read;   #c \"strlen\" \"size_t(const char*)\"\n",
    )
    .unwrap();
    fa::write(
        dir.join("prog.loft"),
        "use cbind;\nfn scripted() -> integer { cbind::c_len(\"hello\") }\n\
         fn main() { println(\"len {scripted()}\") }\n",
    )
    .unwrap();

    let verdict = |toml: &str| -> (bool, String) {
        fa::write(dir.join("loft.toml"), toml).unwrap();
        let out = Command::new(loft_bin())
            .arg("--interpret")
            .arg("--timeout")
            .arg("60")
            .arg("--lib")
            .arg(dir.join("lib"))
            .arg(dir.join("prog.loft"))
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let mut both = String::from_utf8_lossy(&out.stdout).into_owned();
        both.push_str(&String::from_utf8_lossy(&out.stderr));
        (out.status.success(), both)
    };

    const HEAD: &str = "[sandbox]\nmod = [\"fn:scripted\"]\n[profile.mod]\n";

    // 1. The cap is granted and `native_ffi` is unset: REJECTED, and the message
    //    says why the grant cannot admit it.
    let (ok, out) = verdict(&format!(
        "{HEAD}allow_libs = [\"code\"]\nallow = [\"db#read\"]\n"
    ));
    assert!(!ok, "a granted #cap must not admit a `#c` binding: {out}");
    assert!(
        out.contains("`#c` binding")
            && out.contains("strlen")
            && out.contains("native_ffi")
            && out.contains("fix:"),
        "the rejection names the C symbol and the gate that governs it: {out}"
    );
    assert!(
        !out.contains("len 5"),
        "a rejected program must not have run: {out}"
    );

    // 2. The host says yes explicitly: admitted, and it really calls the C.
    let (ok2, out2) = verdict(&format!(
        "{HEAD}allow_libs = [\"code\"]\nallow = [\"db#read\"]\nnative_ffi = true\n"
    ));
    assert!(ok2, "`native_ffi = true` must admit it: {out2}");
    assert!(out2.contains("len 5"), "and the call must work: {out2}");

    // 3. The library is allow-listed wholesale: admitted, UNCHANGED.  That is the
    //    host vetting the library as a unit — the same answer a `#native` bridge
    //    in an allow-listed library already gets, and narrowing it here would be
    //    a different policy change wearing this fix's clothes.
    let (ok3, out3) = verdict(&format!("{HEAD}allow_libs = [\"code\", \"cbind\"]\n"));
    assert!(ok3, "a wholesale-allowed library still admits it: {out3}");
    assert!(out3.contains("len 5"), "and the call must work: {out3}");

    let _ = fa::remove_dir_all(&dir);
}

/// loft#1042 — a designation naming a profile that has no `[profile.<name>]`
/// section is reported as UNDEFINED, not as a profile whose grants happen to be
/// empty.
///
/// The two ask for opposite next moves: an empty grant list says *add grants*,
/// and adding them where the docs implied (`[sandbox]`) produces the identical
/// message a second time, because `allow_libs` is only read under a profile
/// section. That second round is what this diagnostic exists to prevent.
#[test]
fn an_undefined_profile_is_named_as_undefined() {
    // `ui` is designated and never defined — the shape SANDBOX.md's example
    // invited before it carried its `[profile.…]` half.
    let policy = "[sandbox]\nui = [\"fn:scripted\"]\n";
    let prog = "fn scripted() -> integer { mtime(\"x\") }\nfn main() -> integer { scripted() }\n";
    let (ok, err) = run("undef_profile", prog, policy, false);
    assert!(!ok, "a violating sandboxed program must still be rejected");
    assert!(
        err.contains("never DEFINED") && err.contains("[profile.ui]"),
        "the refusal must name the profile as undefined and say where to define it: {err}"
    );
    assert!(
        err.contains("ignored"),
        "…and that grants under `[sandbox]` are ignored, which is the second-round trap: {err}"
    );
}

/// THE CONTROL for the test above: a profile that IS defined and grants nothing
/// is a deliberate deny-all, and still reports its (empty) grant lists. If this
/// ever reported "never DEFINED" the diagnostic would have stopped distinguishing
/// the two cases — which is the whole point of it.
#[test]
fn a_defined_but_empty_profile_still_reports_its_grants() {
    let policy = "[sandbox]\nui = [\"fn:scripted\"]\n[profile.ui]\n";
    let prog = "fn scripted() -> integer { mtime(\"x\") }\nfn main() -> integer { scripted() }\n";
    let (ok, err) = run("empty_profile", prog, policy, false);
    assert!(!ok, "a violating sandboxed program must still be rejected");
    assert!(
        !err.contains("never DEFINED"),
        "a section that EXISTS is a deny-all, not a typo: {err}"
    );
    assert!(
        err.contains("allowed capabilities: []"),
        "and it reports the empty grant list it really has: {err}"
    );
}
