// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN131 — suggestions: tell the author what to write instead.
//!
//! A diagnostic says what is wrong; a fix says what to write instead; the linked feature
//! says why. These guards pin the parts that rot silently — an opt-in that leaks, a
//! condition that cannot name its use, a concept whose door opens onto nothing, and a code
//! with no index entry to grep to.

use loft::file_access as fa;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// A program with one avoidable copy: `src` is named into the struct and used again after,
/// so it survives the copy and could not be moved.  Line 7 is the copy; line 8 is the use.
const AVOIDABLE: &str = "struct Holder { v: vector<integer> }\n\n\
     fn use_it(h: Holder) -> integer { len(h.v) }\n\n\
     fn main() {\n  src = [1, 2, 3];\n  h = Holder { v: src };\n  \
     println(\"{use_it(h)} {len(src)}\");\n}\n";

fn probe(src: &str) -> std::path::PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join("loft_pln131");
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(format!("{}_{n}.loft", std::process::id()));
    fa::write(&path, src).expect("write probe");
    path
}

/// Run `--check` on `src`, with `--explain` when asked; return stdout+stderr.
fn run(src: &str, explain: bool) -> String {
    let path = probe(src);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.args(["--interpret", "--check"]);
    if explain {
        cmd.arg("--explain");
    }
    let out = cmd
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("spawn loft");
    let _ = fa::remove_file(&path);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The copy notice carries the code a fix attaches to.
///
/// The code is the frozen identity and the prose is free, so a suggestion must hang off the
/// former — attaching it to a message string is how a suggestion and its diagnostic drift
/// apart, and a suggestion that has drifted from its diagnostic is misinformation.
#[test]
fn the_copy_notice_carries_its_code() {
    let out = run(AVOIDABLE, false);
    assert!(
        out.contains("advice[avoidable-copy]"),
        "the copy notice must print its code; output:\n{out}"
    );
}

/// Fix lines are OPT-IN. loft is meant to be quiet: the resolutions are worth reading when
/// you are acting on a diagnostic and noise when you are not.
#[test]
fn fix_lines_are_opt_in() {
    let quiet = run(AVOIDABLE, false);
    assert!(
        quiet.contains("avoidable-copy"),
        "the diagnostic itself must still fire without --explain; output:\n{quiet}"
    );
    assert!(
        !quiet.contains("  fix  "),
        "fix lines must not appear without --explain; output:\n{quiet}"
    );
}

/// `--explain` offers BOTH tiers, most-teaching first.
///
/// Ranking is on what a fix opens up, not on how short it is: "build the value in place"
/// introduces an idiom reusable everywhere, while "drop the later use" is a local deletion.
#[test]
fn explain_offers_both_tiers_teaching_first() {
    let out = run(AVOIDABLE, true);
    let Some(build) = out.find("build the value in place") else {
        panic!("the mechanical fix must be offered; output:\n{out}");
    };
    let Some(drop) = out.find("drop the later use") else {
        panic!("the conditional fix must be offered; output:\n{out}");
    };
    assert!(
        build < drop,
        "the fix that teaches an idiom must rank above the local deletion; output:\n{out}"
    );
}

/// The condition NAMES the surviving use, by line.
///
/// This is @PLN131's Q6.1, and it is the difference between a veteran affirming a condition
/// in one second and going hunting for it: the analysis holds the last use as a traversal
/// index, so without carrying its location the honest wording is only "after here".
///
/// `src` is used again on line 8 of the probe — asserting the line, not merely the presence
/// of a condition, is what makes this fail if the location is dropped again.
#[test]
fn the_condition_names_the_surviving_use_by_line() {
    let out = run(AVOIDABLE, true);
    assert!(
        out.contains("needs:"),
        "a conditional fix must state the condition it affirms; output:\n{out}"
    );
    assert!(
        out.contains("`src` is used again at line 8"),
        "the condition must name WHERE the source survives, not just that it does; \
         output:\n{out}"
    );
}

/// Three homes, no repetition: the message says what is WRONG and nothing about the cure.
///
/// The diagnostics used to carry their own resolution inline, so `--explain` printed the same
/// advice twice — the duplication a reader pays for on every diagnostic. What makes this
/// checkable rather than a matter of taste is the fix's own words: if the message contains the
/// imperative the fix line offers, the two homes have merged again.
#[test]
fn the_message_does_not_repeat_its_own_fix() {
    let quiet = run(AVOIDABLE, false);
    for cure in ["build the value in place", "drop the later use"] {
        assert!(
            !quiet.contains(cure),
            "the message repeats its fix (\"{cure}\") — what is wrong belongs to the \
             diagnostic, what to write instead belongs to the fix; output:\n{quiet}"
        );
    }
}

/// Moving the cure out of the message must not leave a reader with LESS than before.
///
/// Fix lines are opt-in, so a plain run now says only what is wrong — and someone who has
/// never heard of `--explain` would simply be told less. One line per RUN closes that, and it
/// has to be per run: a pointer under each diagnostic would double the output on a file with
/// fifty copy notices, which is the noise the opt-in exists to avoid.
#[test]
fn a_quiet_run_says_where_the_fixes_are_exactly_once() {
    let quiet = run(AVOIDABLE, false);
    assert_eq!(
        quiet.matches("re-run with `--explain`").count(),
        1,
        "a quiet run must point at `--explain` exactly once — not per diagnostic, and not \
         never; output:\n{quiet}"
    );
    // …and it must not nag the reader who already asked.
    let explained = run(AVOIDABLE, true);
    assert!(
        !explained.contains("re-run with `--explain`"),
        "the pointer must vanish once the fixes are shown; output:\n{explained}"
    );
}

/// The concept is a handle plus a door, and the door must open onto something real.
///
/// A door onto nothing is worse than no door — so the catalogue entry the fix names has to
/// exist. Checked against the committed snapshot, which is generated from the canonical
/// issue, so deleting or renumbering the feature breaks this rather than shipping a dead
/// link to a reader.
#[test]
fn the_concept_door_resolves_to_a_real_catalogue_entry() {
    let out = run(AVOIDABLE, true);
    assert!(
        out.contains("[move · @F106]"),
        "each fix must name its concept and the entry it opens onto; output:\n{out}"
    );
    let snapshot = fa::read_to_string("index/features.json").expect("features snapshot");
    let found = snapshot.contains("\"number\": 106") || snapshot.contains("\"number\":106");
    assert!(
        found,
        "@F106 is the door the copy fixes open onto — it must exist in the catalogue"
    );
}

// ── steps 3–4: verifying a fix, and applying it ──────────────────────────────

/// A program whose one fix is mechanical, spelled, and placeable: a literal `}` in a
/// format string. Line 2 is the offence.
const BRACE: &str = "fn main() {\n  println(\"a } b\");\n}\n";

/// Run `loft fix` (report) or `loft fix --apply` on a file, returning stdout+stderr.
fn fix_cmd(path: &std::path::Path, apply: bool) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("fix");
    if apply {
        cmd.arg("--apply");
    }
    let out = cmd
        .arg(path)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft fix");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Step 3 — a fix is CHECKED by running it, not by looking plausible.
///
/// The compiler holds the analysis that raised the diagnostic, so a candidate rewrite can
/// be applied to an in-memory copy and the analysis re-run. That is what separates a
/// suggestion that has been tried from one that was pattern-matched, and it is the whole
/// reason `--apply` can be trusted to run unattended.
#[test]
fn a_fix_is_verified_by_running_it() {
    let path = probe(BRACE);
    let out = fix_cmd(&path, false);
    assert!(
        out.contains("double the brace") && out.contains("[verified]"),
        "a mechanical fix that clears its diagnostic must report as verified; output:\n{out}"
    );
    let _ = fa::remove_file(&path);
}

/// Reporting must not write. `loft fix` without `--apply` is a read-only question.
#[test]
fn reporting_a_fix_does_not_touch_the_file() {
    let path = probe(BRACE);
    let before = fa::read_to_string(&path).expect("probe");
    let out = fix_cmd(&path, false);
    let after = fa::read_to_string(&path).expect("probe");
    assert_eq!(
        before, after,
        "`loft fix` without `--apply` must change nothing; output:\n{out}"
    );
    assert!(
        !out.contains("[applied]"),
        "a report that claims an edit it did not make is the one output a reader cannot \
         check; output:\n{out}"
    );
    let _ = fa::remove_file(&path);
}

/// Step 4 — `--apply` writes the fix, and the result actually compiles and runs.
///
/// Asserting the file merely CHANGED would pass for a rewrite that broke the program, which
/// is the failure this feature exists to avoid. So the applied program is run, and its
/// output is the one the author was trying to write: a literal `}`.
#[test]
fn applying_a_fix_produces_a_program_that_runs() {
    let path = probe(BRACE);
    let out = fix_cmd(&path, true);
    assert!(
        out.contains("[applied]"),
        "the mechanical fix must be written; output:\n{out}"
    );
    let src = fa::read_to_string(&path).expect("probe");
    assert!(
        src.contains("a }} b"),
        "the brace must be doubled in place; got:\n{src}"
    );
    let run = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret"])
        .arg(&path)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run the fixed program");
    assert!(
        run.status.success(),
        "the applied fix must leave a program that compiles: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("a } b"),
        "the fixed program must print the literal brace the author wanted: {}",
        String::from_utf8_lossy(&run.stdout)
    );
    let _ = fa::remove_file(&path);
}

/// Applying twice is a no-op — the second run has nothing left to fix.
///
/// An applier that re-applies its own output doubles the brace again on every run, which is
/// the classic way a quick-fix corrupts a file nobody was watching.
#[test]
fn applying_twice_changes_nothing_the_second_time() {
    let path = probe(BRACE);
    fix_cmd(&path, true);
    let once = fa::read_to_string(&path).expect("probe");
    fix_cmd(&path, true);
    let twice = fa::read_to_string(&path).expect("probe");
    assert_eq!(once, twice, "a second `--apply` must find nothing to do");
    let _ = fa::remove_file(&path);
}

/// A rewrite that would introduce a NEW error is refused, not written.
///
/// `x: integer = "5" as integer` is offered the checked cast like any other failing parse,
/// but `as integer?` in that slot yields `integer?` into a non-null `integer` — the fix
/// does not survive its own verification. This is step 3 paying for itself: the suggestion
/// is plausible, the measurement is what says no.
#[test]
fn a_fix_that_would_break_the_program_is_refused() {
    let path = probe("fn main() { x: integer = \"5\" as integer; println(\"{x}\"); }\n");
    let before = fa::read_to_string(&path).expect("probe");
    let out = fix_cmd(&path, true);
    assert!(
        out.contains("REJECTED"),
        "a rewrite that introduces an error must be refused; output:\n{out}"
    );
    assert_eq!(
        before,
        fa::read_to_string(&path).expect("probe"),
        "a refused fix must not reach the file; output:\n{out}"
    );
    let _ = fa::remove_file(&path);
}

/// Nothing to fix, nothing to say — loft is BORING when there is no work.
#[test]
fn a_clean_file_reports_nothing() {
    let path = probe("fn main() { println(\"ok\"); }\n");
    let out = fix_cmd(&path, false);
    assert!(
        out.trim().is_empty(),
        "a file with no fixes must print nothing; output:\n{out}"
    );
    let _ = fa::remove_file(&path);
}

/// A fix that does not hold in every shape must not lead — soundness outranks teaching.
///
/// `as τ?` is the better idiom and it makes the expression NULLABLE, which a target declared
/// non-null rejects. The parser cannot see that target: applying the fix changes what pass 1
/// infers, so only a re-parse knows. It therefore ships as a CONDITION the author can check
/// in a second, ranked below the two discharging forms that hold wherever this diagnostic
/// fires — "prefer the fix that teaches" is a tiebreak between EQUALLY SOUND fixes, never a
/// licence to lead with one that works three times in four.
#[test]
fn a_fix_that_does_not_always_hold_ranks_below_ones_that_do() {
    let out = run(
        "fn main() { x: integer = \"5\" as integer; println(\"{x}\"); }\n",
        true,
    );
    let Some(fallback) = out.find("give the parse a fallback") else {
        panic!("the always-sound discharge must be offered; output:\n{out}");
    };
    let Some(checked) = out.find("make the cast checked") else {
        panic!("the checked cast must still be offered; output:\n{out}");
    };
    assert!(
        fallback < checked,
        "a fix that holds in every shape must rank above one that does not; output:\n{out}"
    );
    assert!(
        out.contains("declared non-null"),
        "the checked cast must state the condition its soundness rests on, so the author \
         can check their own declaration; output:\n{out}"
    );
}

/// A conditional fix WITH an edit: verified, reported, and never written unattended.
///
/// This is the shape the tier split exists for, and until the cast fixes were re-tiered
/// nothing shipped it — the rule was pinned only by a unit test. `loft fix` must say the
/// rewrite works AND that the judgement stays with the author: "you must decide" and "it
/// would not have compiled anyway" are different answers.
#[test]
fn a_conditional_fix_is_verified_but_left_to_the_author() {
    let path = probe("fn main() { x = \"5\" as integer; println(\"{x}\"); }\n");
    let before = fa::read_to_string(&path).expect("probe");
    let out = fix_cmd(&path, true);
    assert!(
        out.contains("make the cast checked") && out.contains("yours to accept"),
        "a conditional fix that verifies must say so, and say it is still yours; \
         output:\n{out}"
    );
    assert_eq!(
        before,
        fa::read_to_string(&path).expect("probe"),
        "`--apply` must not write a conditional fix, however well it verifies; output:\n{out}"
    );
    let _ = fa::remove_file(&path);
}

/// Giving a diagnostic a code must not turn it into a build break.
///
/// `loft test` classified its diagnostics by rendered prefix — `"Advice:"` and `"Warning:"`
/// — and a CODED one renders `Advice[superseded-call]:`, matching neither. Every coded
/// warning and advice therefore fell through to `errors`, and the file failed with
/// "(parse errors)". The trap is that it fires the moment a diagnostic gains its stable
/// identity, which is the one change @PLN131 asks everyone to make: 35 uncoded warnings are
/// queued behind exactly this step.
///
/// Both directions are asserted. Advice must NOT gate even under `--deny-warnings` (the old
/// form keeps working, so ignoring the steer cannot produce a wrong result), and a real
/// warning must still gate — a classifier that files everything as advice would pass the
/// first assertion and break the lint that pays for the feature.
#[test]
fn a_coded_advice_does_not_fail_the_test_runner() {
    let steer = "fn scaled(v: integer, by: integer) -> integer { v * by }\n\
         fn doubled(v: integer) -> integer { scaled(v, 2) }  #superseded \"scaled\"\n\
         fn test_it() { assert(doubled(21) == 42, \"ok\"); }\n";
    let path = probe(steer);
    for deny in [false, true] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
        cmd.args(["--interpret", "--tests"]);
        if deny {
            cmd.arg("--deny-warnings");
        }
        let out = cmd
            .arg(&path)
            .env("LOFT_TIMEOUT", "120")
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("spawn loft --tests");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success() && !all.contains("parse errors"),
            "a coded ADVICE must not fail `loft test` (deny={deny}); output:\n{all}"
        );
    }
    let _ = fa::remove_file(&path);

    // The control: a real warning still gates, so the classifier cannot have been "fixed"
    // by calling everything advice.
    let warn = probe(
        "fn f(v: integer) -> integer { d = v; d = 9; return v; }\n\
         fn test_w() { assert(f(1) == 1, \"ok\"); }\n",
    );
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret", "--tests", "--deny-warnings"])
        .arg(&warn)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft --tests");
    let all = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        all.contains("--deny-warnings:"),
        "a real warning must still gate under --deny-warnings; output:\n{all}"
    );
    let _ = fa::remove_file(&warn);
}

/// The steer names its successor in the message, so its fix writes itself.
#[test]
fn the_superseded_steer_offers_the_successor_as_a_fix() {
    let out = run(
        "fn scaled(v: integer, by: integer) -> integer { v * by }\n\
         fn doubled(v: integer) -> integer { scaled(v, 2) }  #superseded \"scaled\"\n\
         fn main() { println(\"{doubled(21)}\"); }\n",
        true,
    );
    assert!(
        out.contains("advice[superseded-call]"),
        "the steer must carry its code; output:\n{out}"
    );
    assert!(
        out.contains("call `scaled` instead") && out.contains("write `scaled`"),
        "the fix must name the successor AND spell the rewrite; output:\n{out}"
    );
}

/// @PLN131 — `source.fixAll` applies exactly what `loft fix --apply` would, and no more.
///
/// The editor must not become a second implementation of "which fixes are safe": it
/// delegates to `fix_apply::apply_fixes`, so both lanes carry the same three gates
/// (mechanical, spells a placeable edit, verifies). Asserting the OUTPUT rather than the
/// action's presence is what pins that — an editor applying a different set would still
/// produce an action.
#[test]
fn fix_all_matches_what_the_cli_would_write() {
    let src = "fn main() {\n  println(\"a } b\");\n}\n";
    let stdlib = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("default");
    let dir = stdlib.to_string_lossy().to_string();
    let diags = loft::lsp::diagnose(src, "buf.loft", &dir);
    let (rewritten, report) = loft::fix_apply::apply_fixes(src, "buf.loft", &dir, &diags);
    assert_eq!(
        report.iter().filter(|r| r.written).count(),
        1,
        "the brace fix is mechanical, placeable and verifies — it must be written"
    );
    assert!(
        rewritten.contains("a }} b"),
        "fix-all must produce the doubled brace: {rewritten}"
    );

    // …and the same path through the CLI lands the identical bytes on disk.
    let path = probe(src);
    fix_cmd(&path, true);
    assert_eq!(
        fa::read_to_string(&path).expect("probe"),
        rewritten,
        "the editor's fix-all and `loft fix --apply` must not drift"
    );
    let _ = fa::remove_file(&path);
}

/// A conditional fix is never part of fix-all, however applicable it looks.
///
/// Editors bind `source.fixAll` to fix-on-save, so it is the unattended lane by definition —
/// the one place a condition has nobody to affirm it. The cast fix spells a real edit and
/// still must not be written.
#[test]
fn fix_all_leaves_conditional_fixes_alone() {
    let src = "fn main() { x = 1e30 as integer; println(\"{x}\"); }\n";
    let stdlib = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("default");
    let dir = stdlib.to_string_lossy().to_string();
    let diags = loft::lsp::diagnose(src, "buf.loft", &dir);
    let (rewritten, report) = loft::fix_apply::apply_fixes(src, "buf.loft", &dir, &diags);
    assert!(
        report.iter().any(|r| r.title.contains("checked")),
        "the checked-cast fix must be considered at all: {:?}",
        report.iter().map(|r| &r.title).collect::<Vec<_>>()
    );
    assert!(
        report.iter().all(|r| !r.written),
        "no conditional fix may be written unattended"
    );
    assert_eq!(
        rewritten, src,
        "fix-all must leave the buffer untouched here"
    );
}

/// @PLN131 — an UNMASKED error is not one the fix caused.
///
/// `parse_source` returns early when pass 1 errors, so a truncated parse reports no pass-2
/// diagnostic at all. Fixing the pass-1 blocker lets the next parse reach them, and a plain
/// set-difference read every one as damage the rewrite did — which made `--apply` and
/// fix-all refuse any file whose fix was not its ONLY error.
///
/// Both orderings are pinned, and the second is the one that matters: the hidden cast sits
/// ABOVE the brace, so a rule that compared POSITIONS would still reject it. The mechanism
/// is the phase, not the line.
#[test]
fn a_masked_error_does_not_count_against_the_fix() {
    for (name, src) in [
        (
            "hidden below",
            "fn main() {\n  println(\"a } b\");\n  x = 1e30 as integer;\n  println(\"{x}\");\n}\n",
        ),
        (
            "hidden above",
            "fn main() {\n  x = 1e30 as integer;\n  println(\"a } b\");\n  println(\"{x}\");\n}\n",
        ),
    ] {
        let path = probe(src);
        let out = fix_cmd(&path, false);
        assert!(
            out.contains("double the brace") && out.contains("[verified]"),
            "[{name}] a fix must not be blamed for an error it merely UNCOVERED; \
             output:\n{out}"
        );
        let _ = fa::remove_file(&path);
    }
}

/// …and the gate still bites where the rewrite genuinely breaks something.
///
/// This is the assertion that keeps the loosening honest. `x: integer = "5" as integer?`
/// fails in the SAME phase the original reached, so the two parses are comparable and the
/// verdict must still be a refusal. Without this, "ignore new errors" would have quietly
/// become "ignore all errors".
#[test]
fn a_genuinely_broken_rewrite_is_still_refused() {
    let path = probe("fn main() { x: integer = \"5\" as integer; println(\"{x}\"); }\n");
    let before = fa::read_to_string(&path).expect("probe");
    let out = fix_cmd(&path, true);
    assert!(
        out.contains("REJECTED"),
        "a same-phase regression must still be refused; output:\n{out}"
    );
    assert_eq!(
        before,
        fa::read_to_string(&path).expect("probe"),
        "and must not reach the file; output:\n{out}"
    );
    let _ = fa::remove_file(&path);
}

// ── did-you-mean: the typo family becomes an applicable fix ──────────────────

/// @PLN131 — a "did you mean" is applied, and the program then RUNS.
///
/// These four sites already computed a replacement and knew where the name sat, but only an
/// LSP quickfix could reach it: no `--explain` line, no `loft fix`, and — the part that
/// matters — never verified. Asserting the program runs afterwards is the point; a rename
/// that merely changes the text is not a fix.
#[test]
fn a_did_you_mean_is_applied_and_the_program_runs() {
    for (what, src, expect) in [
        (
            "variable",
            "fn main() { value = 1; println(\"{valu}\"); }\n",
            "1",
        ),
        (
            "function",
            "fn helper(v: integer) -> integer { v }\nfn main() { println(\"{helpr(7)}\"); }\n",
            "7",
        ),
        (
            "field",
            "fn main() { s = \"hi\"; println(\"{s.starts_wit(\\\"h\\\")}\"); }\n",
            "true",
        ),
    ] {
        let path = probe(src);
        let out = fix_cmd(&path, true);
        assert!(
            out.contains("[applied]"),
            "[{what}] the rename must be written; output:\n{out}"
        );
        let run = Command::new(env!("CARGO_BIN_EXE_loft"))
            .args(["--interpret"])
            .arg(&path)
            .env("LOFT_TIMEOUT", "120")
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the fixed program");
        let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
        assert!(
            run.status.success() && stdout.contains(expect),
            "[{what}] the renamed program must compile and print {expect}: {stdout}{}",
            String::from_utf8_lossy(&run.stderr)
        );
        let _ = fa::remove_file(&path);
    }
}

/// The spelling is a GUESS; the re-parse is what makes it a measurement.
///
/// `helpr` → `helper` is the right spelling and the wrong call — the suggestion is chosen by
/// edit distance, which knows nothing about arity. Verification catches it, which is the
/// whole reason these can ship as `Mechanical` and be written unattended: a pattern match
/// alone would have edited the file and left it broken.
#[test]
fn a_did_you_mean_that_does_not_fit_is_refused() {
    let path = probe(
        "fn helper(a: integer, b: integer) -> integer { a + b }\n\
         fn main() { println(\"{helpr(1)}\"); }\n",
    );
    let before = fa::read_to_string(&path).expect("probe");
    let out = fix_cmd(&path, true);
    assert!(
        out.contains("REJECTED"),
        "a rename whose call no longer fits must be refused; output:\n{out}"
    );
    assert_eq!(
        before,
        fa::read_to_string(&path).expect("probe"),
        "and must not reach the file; output:\n{out}"
    );
    let _ = fa::remove_file(&path);
}
