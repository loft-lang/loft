// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Doc-hygiene regression guards.
//!
//! QUALITY.md Tier 4 item 10 calls out that INCONSISTENCIES.md,
//! PROBLEMS.md, and CAVEATS.md drift independently — resolved INCs
//! get a status block in their long-form entry but the Summary-by-
//! Severity tables keep listing them as open.  These tests lock the
//! invariant: every INC appearing as "Resolved as design point" must
//! NOT also appear in the Medium or Low severity tables.

use std::fs;

const DOC: &str = "doc/claude/INCONSISTENCIES.md";
const PROBLEMS: &str = "doc/claude/PROBLEMS.md";
const CAVEATS: &str = "doc/claude/CAVEATS.md";
const QUALITY: &str = "doc/claude/QUALITY.md";

/// Every guard added under `tests/scripts/` must record the control it was falsified
/// against, or say why it cannot have one.
///
/// A guard that passes on the build it was written for proves nothing, and the ways that
/// happens are not exotic — four turned up in one afternoon (QUALITY.md § B6m): the wrong
/// ENTRY POINT (`tests/wrap.rs::run_test` runs `main` when the file has one and every
/// zero-parameter function otherwise, so `--interpret` on a `main`-less guard runs no
/// assertion at all), a success marker the error report ECHOES, a leak gate that is monotone
/// so an over-free reads as an improvement, and a cell whose shape never reaches the code
/// path it was written for.  Two defects passed a full green gate the same day and were
/// caught only by a second reader.
///
/// So the requirement is the RECORD, and the tool that produces it is
/// `scripts/falsify.sh <guard.loft> <control-ref>` — it builds the control, runs the guard
/// on both trees through the entry point the corpus runner would pick, and compares six
/// channels apart (exit code, assertion failures, leaked stores, panic, stack-store free
/// refusals, and the guard's own `@EXPECT_ERROR` declarations) so the line names WHICH one
/// moved.  One channel on one backend is enough — a backend-divergence guard cannot move
/// both sides, and its inert side is recorded as expected rather than held against it:
///
/// ```text
/// // @falsified-at: 3ca5ec79 — interpret leaked kt=78 Sk×156 -> clean, native leaked … -> clean
/// ```
///
/// `// @falsified-by: tests/falsified/<guard>.patch` is the same receipt carrying the
/// defect instead of pointing at it — the patch reintroduces the fault on top of HEAD, so
/// nothing outside the repository has to survive for the guard to be re-run.  It is the
/// form for a control that no longer exists anywhere: a squash-merge keeps no branch
/// pointing at the commit a guard was falsified against, and 124 of the 359 receipts on
/// `main` name a build no clone can rebuild (TESTING.md).  `scripts/falsify.sh <guard>
/// --patch <file>` scores one, and refuses when the patch has drifted out of applying —
/// which is the receipt reporting its own staleness, something a dangling sha cannot do.
///
/// `// @falsified-at: none — <reason>` is the honest opt-out for a file that genuinely
/// cannot fail on any earlier build (a corpus that predates what it exercises, a
/// refusal-only file).  Stating the reason is the point: the failure this gate exists to
/// stop is a guard nobody ever watched fail, not a missing string.
///
/// `tests/falsified.baseline` carries the files that predate the requirement.  It is a
/// RATCHET — it only ever shrinks, and a new file must not be added to it.
/// A guard whose receipt does not say how to SCORE it again is defective on its own terms.
///
/// Whether a recorded patch reintroduces THE defect cannot be checked here — it costs an
/// apply, a build and a run per guard, and the answer is a judgement about which channel
/// moved.  That is a per-release READ (`make falsify-review`).  What decides whether that
/// read takes an afternoon or a week is whether each receipt states the four things the
/// reader needs, so this gate holds the line on the DOCUMENTATION while the human keeps the
/// judgement.
///
/// The fields, each earned from a failure that cost real time (TESTING.md): CHANNEL, which
/// channel carries the defect; ARMED, the instrument the measurement needs, asked only of a
/// leak-class guard because an unarmed leak run reports a DIFFERENT channel rather than a
/// weaker one; WITNESS, the concrete observation on the control, which is what makes the
/// re-read quick; and HOLDS, what must NOT move, which is the field that rejects a patch
/// reintroducing some neighbouring fault.
///
/// `tests/falsified_docs.baseline` carries the guards that predate the standard.  Like the
/// falsification ratchet it only ever SHRINKS: a new guard must meet the standard, and a
/// baseline line must go once its guard does.
///
/// The rule itself lives in `scripts/falsify-review.py --check` and nowhere else — a second
/// copy here would drift, and then the gate and the review would disagree about what a
/// receipt owes.
#[test]
fn every_guard_says_how_to_score_it_again() {
    let out = std::process::Command::new("python3")
        .args(["scripts/falsify-review.py", "--check"])
        .output()
        .expect("cannot run scripts/falsify-review.py");
    assert!(
        out.status.success(),
        "falsify-review.py --check failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let thin: std::collections::HashMap<String, String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(n, f)| (n.to_string(), f.to_string()))
        .collect();

    let baseline_src = fs::read_to_string("tests/falsified_docs.baseline")
        .expect("cannot read tests/falsified_docs.baseline");
    let baseline: std::collections::HashSet<&str> = baseline_src
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    let mut added: Vec<String> = thin
        .iter()
        .filter(|(n, _)| !baseline.contains(n.as_str()))
        .map(|(n, f)| format!("  {n} — its receipt does not state: {f}"))
        .collect();
    added.sort();

    let mut fixed: Vec<String> = baseline
        .iter()
        .filter(|b| !thin.contains_key(**b))
        .map(|b| format!("  {b}"))
        .collect();
    fixed.sort();

    assert!(
        added.is_empty() && fixed.is_empty(),
        "the receipt-documentation ratchet slipped:\n{}{}",
        if added.is_empty() {
            String::new()
        } else {
            format!(
                "these receipts do not say how to score them again (see TESTING.md \u{a7} The \
                 patch receipt; `make falsify-review` explains each field):\n{}\n",
                added.join("\n")
            )
        },
        if fixed.is_empty() {
            String::new()
        } else {
            format!(
                "these now meet the standard — delete their lines from \
                 tests/falsified_docs.baseline (the ratchet only shrinks):\n{}\n",
                fixed.join("\n")
            )
        }
    );
}

/// No tracked file carries MERGE-CONFLICT debris.
///
/// `CHANGELOG.md` — the file a USER reads — shipped a bare `=======` and a
/// `>>>>>>> f5cd8a37e (An element read is not more non-null …)` for weeks, and so did
/// `CHANGELOG_TECHNICAL.md`.  They arrived through a squash merge (#1465) and reached `main`,
/// where nothing looked: every gate here reads structure or numbers, and a conflict marker is
/// ordinary text to all of them.  Both halves of the content were present, so no reader of the
/// prose would notice a missing sentence — only the markers themselves said anything was wrong.
///
/// Keyed on `<<<<<<< ` and `>>>>>>> ` WITH the trailing space, never on a bare `=======`: a
/// Setext-style Markdown heading underline is exactly that, and this file's own docs use them.
/// The two chevron forms cannot occur in prose, which is what makes the check total without
/// being a false-positive machine.
#[test]
fn no_tracked_file_carries_conflict_markers() {
    let out = std::process::Command::new("git")
        .args(["grep", "-n", "-I", "-E", "^(<<<<<<< |>>>>>>> )"])
        .current_dir(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .output()
        .expect("git grep runs");
    // `git grep` exits 1 when it matches NOTHING, which is the healthy case here.
    let hits = String::from_utf8_lossy(&out.stdout);
    let hits: Vec<&str> = hits.lines().filter(|l| !l.is_empty()).collect();
    assert!(
        hits.is_empty(),
        "{} line(s) of merge-conflict debris are committed — a resolution that kept both halves \
         of the text and left the markers behind reads as ordinary prose to every other gate:\n{}",
        hits.len(),
        hits.join("\n")
    );
}

#[test]
fn every_new_guard_records_its_control() {
    let baseline_src = fs::read_to_string("tests/falsified.baseline")
        .expect("cannot read tests/falsified.baseline");
    let baseline: std::collections::HashSet<&str> = baseline_src
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    let mut missing: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in fs::read_dir("tests/scripts").expect("cannot read tests/scripts") {
        let path = entry.expect("bad dir entry").path();
        if path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("loft"))
        {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("non-utf8 script name")
            .to_string();
        let src = fs::read_to_string(&path).unwrap_or_default();
        let records = src.contains("@falsified-at:") || src.contains("@falsified-by:");
        if records && baseline.contains(name.as_str()) {
            // Retrofitted: the baseline line is now the stale half.
            missing.push(format!(
                "  {name} records a control AND is still in tests/falsified.baseline —                  delete its baseline line (the ratchet only shrinks)"
            ));
        } else if !records && !baseline.contains(name.as_str()) {
            missing.push(format!(
                "  {name} has no `// @falsified-at:` line — run                  `scripts/falsify.sh tests/scripts/{name} <control-ref>` and paste what it                  prints, or record `// @falsified-at: none — <reason>`"
            ));
        }
        seen.insert(name);
    }
    let vanished: Vec<&str> = baseline
        .iter()
        .filter(|b| !seen.contains(**b))
        .copied()
        .collect();

    assert!(
        missing.is_empty() && vanished.is_empty(),
        "the falsification ratchet slipped:\n{}{}",
        missing.join("\n"),
        if vanished.is_empty() {
            String::new()
        } else {
            format!(
                "\n  these are in tests/falsified.baseline but no longer exist: {}",
                vanished.join(", ")
            )
        }
    );
}

/// Every `#[ignore = "..."]` — and every `#[cfg_attr(<cfg>, ignore = "...")]` — in
/// `tests/*.rs` and `src/**/*.rs` must match the committed baseline at
/// `tests/ignored_tests.baseline`, keyed `<file>::<test>` because two `src/` tests share
/// a bare name.  A drift is one of three things worth the author's attention at review:
///   * an ignored test just got its fix landed — delete its line and un-ignore it;
///   * a new ignored test was added — the baseline grows, with the reason it carries;
///   * the reason string was edited — the baseline updates.
///
/// Without this guard, silently-passing ignored tests and stale reason strings
/// accumulate invisibly — and until 2026-09 the guard read ONE file, `tests/issues.rs`,
/// so the release checklist's `A-ignores` vouched for 1 ignore while nextest was skipping
/// 35.  `make release-checklist` reads the same baseline, which is why a reason-less
/// ignore is a release finding and not only a lint.
/// Regenerate the baseline with
/// `python3 tests/dump_ignored_tests.py > tests/ignored_tests.baseline`.
#[test]
fn ignored_tests_baseline_is_current() {
    fn rust_files(dir: &str, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                rust_files(&p.to_string_lossy(), out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut files: Vec<String> = Vec::new();
    for e in fs::read_dir("tests").expect("read tests/").flatten() {
        let p = e.path();
        if p.is_file() && p.extension().is_some_and(|x| x == "rs") {
            files.push(p.to_string_lossy().replace('\\', "/"));
        }
    }
    rust_files("src", &mut files);
    files.sort();

    let mut actual: Vec<(String, String)> = Vec::new();
    for file in &files {
        let src = fs::read_to_string(file).unwrap_or_else(|e| panic!("cannot read {file}: {e}"));
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            // `#[ignore…]` or `#[cfg_attr(<cfg>, ignore…)]` — both carry a reason the same way.
            let rest = if let Some(r) = t.strip_prefix("#[ignore") {
                r
            } else if t.starts_with("#[cfg_attr(") && t.contains("ignore") {
                &t[t.find("ignore").unwrap() + "ignore".len()..]
            } else {
                continue;
            };
            let reason = match rest.trim_start().strip_prefix('=') {
                Some(r) => {
                    let Some(start) = r.find('"') else { continue };
                    let tail = &r[start + 1..];
                    let Some(end) = tail.rfind('"') else { continue };
                    tail[..end].replace("\\\"", "\"").replace("\\\\", "\\")
                }
                None => String::new(), // a bare `#[ignore]`: no reason, and the baseline says so
            };
            // Scan forward up to 10 lines for the `fn NAME(` line.
            for next in lines.iter().skip(i + 1).take(10) {
                let nt = next.trim_start();
                let nt = nt
                    .strip_prefix("pub ")
                    .map_or(nt, |r| r.trim_start_matches(|c| c != ' ').trim_start());
                if let Some(after_fn) = nt.strip_prefix("fn ")
                    && let Some(paren) = after_fn.find(['(', '<'])
                {
                    actual.push((format!("{file}::{}", &after_fn[..paren]), reason.clone()));
                    break;
                }
            }
        }
    }
    actual.sort();
    let baseline = fs::read_to_string("tests/ignored_tests.baseline")
        .expect("cannot read tests/ignored_tests.baseline");
    let mut expected: Vec<(String, String)> = Vec::new();
    for line in baseline.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let (name, reason) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("malformed baseline line (missing TAB): `{line}`"));
        expected.push((name.to_string(), reason.to_string()));
    }
    if actual != expected {
        let mut msg =
            String::from("the #[ignore] set drifted from tests/ignored_tests.baseline.\n");
        let actual_set: std::collections::BTreeSet<_> = actual.iter().collect();
        let expected_set: std::collections::BTreeSet<_> = expected.iter().collect();
        for added in actual_set.difference(&expected_set) {
            msg += &format!("  + {}\t{}\n", added.0, added.1);
        }
        for removed in expected_set.difference(&actual_set) {
            msg += &format!("  - {}\t{}\n", removed.0, removed.1);
        }
        msg +=
            "Regenerate with: python3 tests/dump_ignored_tests.py > tests/ignored_tests.baseline";
        panic!("{msg}");
    }
}

/// Every `#[test]` attribute in a test file must be immediately
/// followed by either another `#[…]` attribute or a `fn`.  An
/// orphan `#[test]` — left after moving / reshaping a test block —
/// produces a `duplicate_macro_attributes` warning on the *next*
/// test's attribute and makes test output confusing (the real
/// test name shown is right, but the warning blames an innocent
/// line).  Misled the author into opening a QUALITY.md bug
/// about `code!()` once; this guard prevents it reoccurring.
#[test]
fn no_orphan_test_attributes_in_tests_issues_rs() {
    let path = "tests/issues.rs";
    let src = fs::read_to_string(path).unwrap_or_else(|_| panic!("cannot read {path}"));
    let lines: Vec<&str> = src.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed != "#[test]" {
            continue;
        }
        // Scan forward through blank lines, other attributes, and
        // `///` / `//` doc/comment lines until we find the next
        // non-trivial line.  It must be either another `#[…]`
        // attribute or a `fn` declaration.
        let next = lines
            .iter()
            .enumerate()
            .skip(i + 1)
            .find(|(_, l)| {
                let t = l.trim_start();
                !t.is_empty() && !t.starts_with("//")
            })
            .map(|(j, l)| (j, l.trim_start()));
        match next {
            Some((_, t))
                if t.starts_with("#[") || t.starts_with("fn ") || t.starts_with("pub fn ") => {}
            Some((j, t)) => panic!(
                "Orphan #[test] attribute at {path}:{} — next non-comment line is {path}:{} `{t}` (expected another `#[…]` or `fn`). Remove the stray attribute.",
                i + 1,
                j + 1
            ),
            None => panic!(
                "Orphan #[test] attribute at {path}:{} with no following declaration.",
                i + 1
            ),
        }
    }
}

fn read_doc() -> String {
    fs::read_to_string(DOC).unwrap_or_else(|_| panic!("cannot read {DOC}"))
}

/// QUALITY Tier 2 #4 — once the `cargo clippy --no-default-features
/// --all-targets -- -D warnings` gate goes green, CI must run it on
/// every push so the ratchet can't slip.  This guard reads the
/// `Makefile` `ci:` target and asserts the `--no-default-features`
/// clippy invocation is present.  Caught silently-regressed gates
/// before when the `--tests` variant was the only one listed.
#[test]
fn ci_target_runs_no_default_features_clippy() {
    let makefile = fs::read_to_string("Makefile").expect("cannot read Makefile");
    let needle = "cargo clippy --no-default-features --all-targets -- -D warnings";
    assert!(
        makefile.contains(needle),
        "Makefile `ci:` target must invoke `{needle}` so the --no-default-features clippy ratchet is enforced on every push.  See QUALITY.md Tier 2 #4."
    );
}

/// The `ci:` verdict must hang off ONE `&&` chain, so that `CI-RESULT: ALL GATES PASSED`
/// is a statement about every phase and not just the last one.
///
/// It was not.  Two bookkeeping lines inside the chain were `;`-separated
/// (`gates=…; jobs=…; export …;`), and a `;` TERMINATES the `&&` list before it — so the
/// shell saw two independent lists: everything from `fmt` through `cache warm`, whose exit
/// status was discarded, and then the nextest phase, which was the only thing the verdict
/// ever reported.  A failing `fmt` did not merely fail to fail the gate; it skipped clippy,
/// the doc-drift check, the label guards, all five builds and the target-surface check, and
/// the run went straight to the tests and printed ALL GATES PASSED (loft#1267 session,
/// CI_BUDGET.md § `CI-RESULT` measured only the TEST phase).
///
/// The shape to keep is a brace group joined with `&&`: `{ gates=…; …; } && \`.  This guard
/// reads the recipe and refuses a bare `;`-terminated continuation line, which is the only
/// spelling that can silently re-split the chain.
#[test]
fn ci_target_verdict_covers_every_phase_not_just_the_tests() {
    let makefile = fs::read_to_string("Makefile").expect("cannot read Makefile");
    let start = makefile
        .find("\nci: ci-guard\n")
        .expect("Makefile must define the `ci:` target");
    // The recipe ends at the first line that is neither a comment nor tab-indented.
    let body: String = makefile[start + 1..]
        .lines()
        .skip(1)
        .take_while(|l| l.starts_with('\t') || l.trim_start().starts_with('#') || l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        body.contains("cargo nextest run --profile ci"),
        "the `ci:` recipe was not captured — the extractor above needs updating"
    );
    // Only continuation lines matter: a line ending in `\` is spliced into the chain, so a
    // `;` at its end terminates the `&&` list the verdict hangs off.  A line that does NOT
    // continue is its own recipe line and cannot split anything.
    //
    // A `;` INSIDE a brace group is the cure, not the disease, so track group depth and
    // judge only lines that end at depth 0.  Counting raw braces is sound here because the
    // other brace users in this recipe — `$${gates:-1}` and friends — are balanced within
    // their own line.
    let mut depth: i32 = 0;
    let mut offenders: Vec<&str> = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim_end();
        let continues = trimmed.ends_with('\\');
        let code = trimmed.trim_end_matches('\\').trim_end();
        depth += i32::try_from(code.matches('{').count()).unwrap_or(0)
            - i32::try_from(code.matches('}').count()).unwrap_or(0);
        if continues && depth == 0 && code.ends_with(';') {
            offenders.push(line);
        }
    }
    assert!(
        offenders.is_empty(),
        "a continuation line in the `ci:` chain ends in `;`, which terminates the `&&` list \
         the verdict hangs off — every phase before it becomes unreported.  Wrap the \
         bookkeeping in a brace group joined with `&&`: `{{ gates=…; …; }} && \\`.\n\
         Offending line(s):\n{}",
        offenders.join("\n")
    );
}

/// QUALITY Tier 1 #3 — `p122_long_running_struct_loop` was ignored
/// only because it takes ~10 min in debug and ~0.05 s in release,
/// not because the test itself was broken.  Closed 2026-04-14 by
/// switching the attribute to `#[cfg_attr(debug_assertions, ignore)]`
/// so release builds (CI's default) run it automatically.  This
/// guard locks the debug-only ignore form in place: a future edit
/// that reverts to plain `#[ignore]` would silently re-remove the
/// stress regression from CI.
#[test]
fn p122_long_running_struct_loop_is_cfg_attr_ignored_in_debug_only() {
    let src = fs::read_to_string("tests/issues.rs").expect("cannot read tests/issues.rs");
    let idx = src
        .find("fn p122_long_running_struct_loop")
        .expect("tests/issues.rs must define p122_long_running_struct_loop");
    // Look backwards ~30 lines to capture the attribute stack above the fn.
    let preamble_start = src[..idx]
        .rfind("#[test]")
        .unwrap_or(idx.saturating_sub(30 * 120));
    let preamble = &src[preamble_start..idx];
    assert!(
        preamble.contains("cfg_attr(")
            && preamble.contains("debug_assertions")
            && preamble.contains("ignore"),
        "p122_long_running_struct_loop must be gated with \
         `#[cfg_attr(debug_assertions, ignore = …)]` so CI (release) \
         runs it.  See QUALITY.md Tier 1 #3.  Preamble found:\n{preamble}"
    );
}

/// QUALITY Tier 3 #8 — the const-store mmap path is intentionally
/// deferred per [plans/82-const-store § Phase B] (cache files are
/// 5-10 KB; mmap overhead exceeds memcpy savings at this size).
/// QUALITY.md must reflect that decision, not ask for a benchmark the
/// design has ruled out.  This guard locks the two docs together: if
/// 82-const-store stops marking Phase B as deferred, QUALITY.md Tier 3
/// #8 should be re-opened at the same time; if QUALITY.md loses the
/// closure marker, 82-const-store probably did too.
#[test]
fn quality_const_store_mmap_matches_const_store_md() {
    let cs_path = "doc/claude/plans/82-const-store/README.md";
    let cs = fs::read_to_string(cs_path).unwrap_or_else(|e| panic!("cannot read {cs_path}: {e}"));
    assert!(
        cs.contains("## Phase B — Memory-mapped constant store (deferred)"),
        "{cs_path} must keep its `## Phase B — Memory-mapped constant store (deferred)` heading — if the design position has changed, re-open QUALITY.md Tier 3 #8 and update this guard."
    );

    let q = read_quality();
    // Locate the Tier 3 #8 block heading.  Items are numbered
    // `8. **...` at column 0; find the one that mentions const-store
    // mmap (not the P54-section "8." which is deeper-indented).
    let heading_start = q
        .find("\n8. **")
        .map(|i| i + 1)
        .expect("QUALITY.md must contain a top-level Tier 3 item `8. **...`");
    let block_end = q[heading_start..]
        .find("\n9. ")
        .unwrap_or(q.len() - heading_start)
        + heading_start;
    let block = &q[heading_start..block_end];
    assert!(
        block.contains("Const store mmap"),
        "QUALITY.md Tier 3 #8 should be about the const-store mmap path.  Heading found:\n{}",
        &block[..block.find('\n').unwrap_or(block.len())]
    );
    assert!(
        block.starts_with("8. **~~") || block.contains("**~~Const store mmap"),
        "QUALITY.md Tier 3 #8 must be struck-through (`~~Const store mmap…~~`) because plans/82-const-store § Phase B is deferred-by-design.  Block:\n{block}"
    );
    assert!(
        block.contains("82-const-store") || block.contains("CONST_STORE.md"),
        "QUALITY.md Tier 3 #8 must reference plans/82-const-store § Phase B (or the legacy CONST_STORE.md path during transition) as the source of the deferral decision.  Block:\n{block}"
    );
}

/// P54 / Q2 / Q3 / Q4 native-registration guard.  The existing
/// `tests/issues.rs::native_rs_functions_up_to_date` only checks
/// functions that carry a `#rust "..."` annotation in `default/`;
/// the P54 JSON family ships pure-native declarations like
/// `pub fn json_null() -> JsonValue;` with no `#rust` body, so
/// their wiring through `NATIVE_FNS` has no automated guard.  A
/// future edit that deletes an entry from `NATIVE_FNS` without
/// removing the loft declaration would leave the declaration
/// "declared but not implemented" — callers get a silent runtime
/// panic rather than a compile-time error.
///
/// This guard enumerates every `pub fn <name>(…) -> <T>;` header
/// in `default/06_json.loft` (body-less declarations = pure
/// natives) and asserts `"n_<name>"` appears as a key in the
/// `NATIVE_FNS` array in `src/native.rs`.  Covers every Q2 / Q3
/// / Q4 helper shipped on the P54 branch.
#[test]
fn p54_json_natives_registered_for_every_declaration() {
    let stdlib =
        fs::read_to_string("default/06_json.loft").expect("cannot read default/06_json.loft");
    let native = fs::read_to_string("src/native.rs").expect("cannot read src/native.rs");
    let mut missing: Vec<String> = Vec::new();
    // Walk lines; a body-less declaration is `pub fn <name>(…) -> <T>;`
    // ending in `;`.  Declarations with `{` start a loft-side
    // implementation (not a native), skip those.
    for line in stdlib.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("pub fn ") else {
            continue;
        };
        let Some(paren) = rest.find('(') else {
            continue;
        };
        let name = &rest[..paren];
        if name.is_empty() {
            continue;
        }
        // Must end with `;` on the same line (no body).  Multi-line
        // signatures aren't used in 06_json.loft today; if that
        // changes, extend this to join continuations before checking.
        if !rest.trim_end().ends_with(';') {
            continue;
        }
        // @PLN10 F — a JSON declaration is satisfied by EITHER its base native
        // (`"n_foo"`) OR a destination-passing variant (`…foo_dest"`).  The base
        // text producers were deleted once codegen routed every call position
        // through `_dest` (synth-dest); the `_dest` impl is now the real binding.
        let needle = format!("\"n_{name}\"");
        let dest_needle = format!("{name}_dest\"");
        if !native.contains(&needle) && !native.contains(&dest_needle) {
            missing.push(name.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "default/06_json.loft declares {} pure-native fn(s) without a matching NATIVE_FNS entry in src/native.rs:\n  {}\n\
         For each missing name `foo`, add `(\"n_foo\", n_foo)` (or a `…foo_dest` variant) to NATIVE_FNS and implement it.  See QUALITY.md § P54 for the JSON native surface.",
        missing.len(),
        missing.join("\n  ")
    );
}

/// QUALITY P54 — the JSON stdlib + native doc-comments must not
/// contain stale "step 4" / "stub" / "forward-compatible" /
/// "lands when X" / "today returns JNull" language now that
/// step 4 is complete.  Earlier drafts of `default/06_json.loft`
/// and `src/native.rs` peppered the surface with forward-
/// compatible-stub callouts so callers could write the API shape
/// ahead of the implementation; once the implementations landed
/// those notes turned into wrong claims about today's behaviour.
///
/// This guard catches those references so they get rewritten
/// alongside the next big landing instead of misleading readers.
/// Both files are scanned because the same staleness shape
/// appeared on both sides of the loft/Rust boundary.
///
/// The check is intentionally narrow: only the strings that
/// appeared in pre-step-4 stub language.  General "step N"
/// references in module-level prose are still allowed (the
/// active-sprint headline at the top of the file points at the
/// roadmap legitimately).
#[test]
fn json_stdlib_has_no_stale_stub_language() {
    let stdlib =
        fs::read_to_string("default/06_json.loft").expect("cannot read default/06_json.loft");
    let native = fs::read_to_string("src/native.rs").expect("cannot read src/native.rs");
    let mut findings: Vec<(String, usize, String)> = Vec::new();
    let stale_phrases = [
        "forward-compatible stub",
        "forward-compat stub",
        "back from the stub",
        "until p54 step 4",
        "lands with p54 step 4",
        "pending a q4 follow-up",
        "step 3 (landing next commit)",
        "today returns an empty",
        "today this returns an empty",
        "today returns `jnull`",
        "non-empty input returns `jnull`",
        // src/native.rs-side variants
        "step 3 stub",
        "still returns 0 (arena materialisation",
        "stored on the call today but unused",
        "ahead of p54 step 4's container materialisation",
    ];
    for (file, src) in [
        ("default/06_json.loft", &stdlib),
        ("src/native.rs", &native),
    ] {
        // For src/native.rs, restrict the scan to the JSON-related
        // region so unrelated `///` comments aren't flagged.  The
        // JSON natives live between the `n_json_parse` start marker
        // and the trailing native-fns block.
        let (start, end) = if file == "src/native.rs" {
            let s = src.find("fn n_json_parse").unwrap_or(0);
            let e = src[s..]
                .find("// ===")
                .map(|off| s + off)
                .unwrap_or(src.len());
            (s, e)
        } else {
            (0, src.len())
        };
        let region = &src[start..end];
        // Map region byte offset back to absolute line numbers by
        // counting lines in the prefix.
        let line_offset = src[..start].lines().count();
        for (i, line) in region.lines().enumerate() {
            let trimmed = line.trim_start();
            // Both `///` (stdlib + Rust doc-comments) and `//!`
            // (Rust module-level) count.
            if !trimmed.starts_with("///") && !trimmed.starts_with("//!") {
                continue;
            }
            let payload = trimmed
                .trim_start_matches("///")
                .trim_start_matches("//!")
                .trim();
            let lower = payload.to_ascii_lowercase();
            for phrase in stale_phrases {
                if lower.contains(phrase) {
                    findings.push((file.to_string(), line_offset + i + 1, payload.to_string()));
                    break;
                }
            }
        }
    }
    assert!(
        findings.is_empty(),
        "JSON natives contain {} stale stub-language doc-comment line(s) (post-P54-step-4 — the implementations have landed, the comments need to flip to describe today's behaviour):\n  {}\n\
         For each finding, rewrite the doc-comment to describe what the function does TODAY (post-step-4 arena materialisation, post-Q2/Q3/Q4 ships) and remove the forward-compatible-stub callouts.",
        findings.len(),
        findings
            .iter()
            .map(|(f, n, p)| format!("{f}:{n}: {p}"))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// loft#851 — every file operation asks ONE question about the browser, and it
/// is the `host_fs` cfg.
///
/// This guard replaces a Tier 3 #9 one that asserted the opposite: that
/// `src/state/io.rs` and `src/database/io.rs` each carry an explicit
/// `target_arch = "wasm32"` branch STUBBING file operations, because `--html`
/// had no reachable filesystem. It does now, so the stubs are gone — but the
/// hazard the old guard was pointing at is real and unchanged: these arms drift.
/// They drifted for a year. The interpreter's browser branches were real host
/// bridges while `codegen_runtime.rs`'s were stubs returning nothing, so the two
/// backends had different filesystems and `--html` — which runs the generated
/// code — had the empty one.
///
/// A hand-written `feature = "wasm"` on a file site is how that happens: the
/// wasm-bindgen bundle and `--html` are both browsers and both reach the host,
/// but only one of them sets that feature. `host_fs` (`build.rs`) names the
/// question once. So the rule is per-site: a file arm may not gate on the
/// wasm-bindgen FEATURE.
#[test]
fn file_operations_gate_on_host_fs_not_the_wasm_feature() {
    for path in [
        "src/state/io.rs",
        "src/database/io.rs",
        "src/codegen_runtime.rs",
    ] {
        let src = fs::read_to_string(path).unwrap_or_else(|_| panic!("cannot read {path}"));
        assert!(
            src.contains("host_fs"),
            "{path} must gate its file operations on the `host_fs` cfg — the one name for \
             \"this build reaches the filesystem through a JS host\" (loft#851)."
        );
        // `feature = "wasm"` still has honest uses here: `src/codegen_runtime.rs`
        // reads the CLOCK differently under wasm-bindgen, which is a real
        // difference between the two browsers rather than a file question. So the
        // check is scoped to lines that also mention a file operation.
        let strays: Vec<String> = src
            .lines()
            .enumerate()
            .filter(|(_, l)| l.contains("cfg") && l.contains("feature = \"wasm\""))
            .filter(|(n, _)| {
                // Look at what the arm guards, not at the attribute alone.
                let window: String = src.lines().skip(*n).take(4).collect::<Vec<_>>().join(" ");
                [
                    "File",
                    "file",
                    "fs_",
                    "read_bytes",
                    "write_bytes",
                    "std::fs",
                ]
                .iter()
                .any(|k| window.contains(k))
            })
            .map(|(n, l)| format!("{path}:{}: {}", n + 1, l.trim()))
            .collect();
        assert!(
            strays.is_empty(),
            "a file operation must not gate on the wasm-bindgen FEATURE — `--html` is a \
             browser too and does not set it, which is exactly how the two backends ended \
             up with different filesystems (loft#851).  Use the `host_fs` cfg:\n  {}",
            strays.join("\n  ")
        );
    }
}

/// QUALITY Tier 4 #11 — DESIGN_DECISIONS.md is the closed-by-decision
/// register.  Without a prominent cross-ref at the top of the docs
/// where new items naturally land (PLANNING.md for future work,
/// PROBLEMS.md for bug reports), the same declined proposals keep
/// coming back every quarter.  This guard asserts both files link
/// to DESIGN_DECISIONS.md near their top — inside the first ~80
/// lines where the header / intro lives, not buried in a
/// cross-references section at the bottom where nobody sees it.
#[test]
fn planning_and_problems_link_to_design_decisions() {
    for (path, label) in [
        ("doc/claude/PLANNING.md", "PLANNING.md"),
        ("doc/claude/PROBLEMS.md", "PROBLEMS.md"),
    ] {
        let src = fs::read_to_string(path).unwrap_or_else(|_| panic!("cannot read {path}"));
        let head: String = src.lines().take(80).collect::<Vec<_>>().join("\n");
        assert!(
            head.contains("DESIGN_DECISIONS.md"),
            "{label} must link to DESIGN_DECISIONS.md in its opening ~80 lines so contributors see the closed-by-decision register before adding a new entry.  See QUALITY.md Tier 4 #11."
        );
    }
}

/// QUALITY Tier 4 #12 — `make ship` is the canonical pre-push gate.
/// Four invariants must all be present in the recipe so users who
/// `make ship && git push` get the same guarantee the remote CI
/// applies.  If any step is dropped the ratchet silently weakens
/// (the exact scenario that produced this sprint's hygiene commits
/// when `cargo clippy --no-default-features` started failing
/// unnoticed).
///
/// Required command fragments, in order:
///   1. `cargo fmt --all -- --check`                             (formatting)
///   2. `cargo clippy --release --all-targets -- -D warnings`    (default features)
///   3. `cargo clippy --no-default-features --all-targets -- -D warnings` (ndf)
///   4. `cargo test --release`                                   (release tests)
///
/// This guard checks the `ship:` recipe contains all four fragments
/// and that they appear in chained order (`&&`) so a failure in step
/// N prevents the later steps (and a subsequent `git push`) from
/// running.
#[test]
fn ship_target_chains_all_required_gates() {
    let makefile = fs::read_to_string("Makefile").expect("cannot read Makefile");
    let ship_recipe = extract_recipe(&makefile, "ship:")
        .expect("Makefile must define a `ship:` target — see QUALITY.md Tier 4 #12");
    let required: &[&str] = &[
        "cargo fmt --all -- --check",
        // CI's exact Clippy line (ci.yml `clippy` job) — `ship` mirrors it
        // verbatim so a local pass can never diverge from the remote gate
        // (the narrower `--release --all-targets` variant passed locally
        // while CI failed on `--all-features`, costing a full CI round).
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo clippy --no-default-features --all-targets -- -D warnings",
        "cargo test --release",
    ];
    let mut cursor = 0usize;
    for frag in required {
        let found = ship_recipe[cursor..].find(frag).unwrap_or_else(|| {
            panic!(
                "`make ship` recipe is missing or mis-orders `{frag}`.  Full recipe:\n{ship_recipe}"
            )
        });
        cursor += found + frag.len();
    }
    assert!(
        ship_recipe.contains("&&"),
        "`make ship` recipe must chain its gates with `&&` so the first failure aborts the chain (and any subsequent `git push` via `make ship && git push`).  Full recipe:\n{ship_recipe}"
    );
}

/// Read the recipe lines of a Makefile target by name.  Returns the
/// concatenated recipe body (the lines starting with TAB after the
/// target header) up to the next blank line or next target.
fn extract_recipe(makefile: &str, target_header: &str) -> Option<String> {
    let mut lines = makefile.lines();
    for line in lines.by_ref() {
        if line.starts_with(target_header) {
            break;
        }
    }
    let mut out = String::new();
    for line in lines {
        // Recipe lines begin with a tab; a non-tab, non-empty line
        // ends the target.
        if line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    if out.is_empty() { None } else { Some(out) }
}

fn read_problems() -> String {
    fs::read_to_string(PROBLEMS).unwrap_or_else(|_| panic!("cannot read {PROBLEMS}"))
}

fn read_caveats() -> String {
    fs::read_to_string(CAVEATS).unwrap_or_else(|_| panic!("cannot read {CAVEATS}"))
}

fn read_quality() -> String {
    fs::read_to_string(QUALITY).unwrap_or_else(|_| panic!("cannot read {QUALITY}"))
}

/// The main "Open programmer-biting issues" table in QUALITY.md must
/// not contain any row whose issue ID is wrapped in `~~…~~` (i.e. a
/// row marked as closed).  Closed items belong in the paragraph
/// below ("Items that look open in the historical sections ...") or
/// in CHANGELOG.md — not in the live queue.  This is the analogue of
/// the PROBLEMS.md Quick-Reference / long-form guard, applied to
/// QUALITY.md's own main table.
///
/// Also checks that every `~~strikethrough~~` item in the Tier 2
/// enhancement list carries a "Landed YYYY-MM-DD" body marker — the
/// convention this session established for 6a/6b/6c/6d.  Without
/// this guard, the project can claim credit for closing an item
/// whose landing date isn't recorded.
#[test]
fn quality_open_table_has_no_crossed_out_rows() {
    let src = read_quality();
    let (_, rest) = src
        .split_once("## Open programmer-biting issues")
        .expect("'## Open programmer-biting issues' heading missing — QUALITY.md layout changed");
    let (table, _after) = rest
        .split_once("\n---")
        .expect("main table not terminated by `---` — QUALITY.md layout changed");
    for line in table.lines() {
        let l = line.trim_start();
        if !l.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        let first = cells[1];
        if first.is_empty() || first == "#" || first.contains("---") {
            continue;
        }
        assert!(
            !first.starts_with("~~"),
            "QUALITY.md main open-issues table has a crossed-out row `{first}` — move it to the closed paragraph below the table (or to CHANGELOG.md)"
        );
    }
}

/// Every `~~…~~` strikethrough Tier-2 sub-item (6a / 6b / 6c / 6d and
/// their future siblings) must carry a `Landed YYYY-MM-DD` marker in
/// its body, linking the crossed-out claim to an audit trail.
#[test]
fn quality_struck_tier2_items_have_landing_date() {
    let src = read_quality();
    let tier2 = src
        .split("### Tier 2")
        .nth(1)
        .expect("### Tier 2 section missing");
    let tier2 = tier2
        .split("### Tier 3")
        .next()
        .expect("### Tier 2 section not terminated by ### Tier 3");
    // Split into item blocks at lines that look like `6a.` / `6b.` / `6c.`
    // / etc. (digit-then-letter then period).
    let mut blocks: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let bytes = tier2.as_bytes();
    let mut i = 0;
    while i + 3 < bytes.len() {
        // A block starts at column 0 with `Nx.` where N is a digit and
        // x is a lowercase letter.
        if (i == 0 || bytes[i - 1] == b'\n')
            && bytes[i].is_ascii_digit()
            && bytes[i + 1].is_ascii_lowercase()
            && bytes[i + 2] == b'.'
        {
            if i > start {
                blocks.push(&tier2[start..i]);
            }
            start = i;
        }
        i += 1;
    }
    if start < tier2.len() {
        blocks.push(&tier2[start..]);
    }
    for block in blocks {
        let first_line = block.lines().next().unwrap_or("");
        if !first_line.contains("~~") {
            continue;
        }
        assert!(
            block.contains("Landed") || block.contains("landed"),
            "Struck-through Tier-2 item `{}` has no 'Landed YYYY-MM-DD' marker in its body — add the landing date or remove the strikethrough",
            first_line.trim()
        );
    }
}

/// Every INC# listed in the "Resolved as design point" table must
/// be absent from the Medium and Low severity tables above it.
#[test]
fn inconsistencies_resolved_inc_not_also_listed_as_open() {
    let src = read_doc();
    let (open, resolved) = src
        .split_once("### Resolved as design point")
        .expect("Resolved-as-design-point section missing");
    let resolved_nrs = inc_numbers_in_table(resolved);
    let open_nrs = inc_numbers_in_table(open);
    for n in &resolved_nrs {
        assert!(
            !open_nrs.contains(n),
            "INC#{n} appears in both Resolved table and an open severity table — update the Summary section"
        );
    }
    assert!(
        !resolved_nrs.is_empty(),
        "Resolved-as-design-point table is empty — unexpected; at least INC#3/#9/#12/#17/#26/#29 should be listed"
    );
}

/// Every INC# with a long-form "**Status (YYYY-MM-DD):**" block must
/// appear in the Resolved-as-design-point table (and not in the open
/// severity tables).  Locks the drift where a status block is added
/// but the summary isn't updated.
#[test]
fn inconsistencies_status_blocks_listed_as_resolved() {
    let src = read_doc();
    let mut status_incs: Vec<u32> = Vec::new();
    for (i, line) in src.lines().enumerate() {
        if line.starts_with("## ")
            && let Some(n) = inc_number_from_heading(line)
        {
            // Scan the next ~60 lines (until the next `## ` heading)
            // for a `**Status (` marker.
            let tail: Vec<&str> = src.lines().skip(i + 1).take(80).collect();
            let has_status = tail
                .iter()
                .take_while(|l| !l.starts_with("## "))
                .any(|l| l.contains("**Status ("));
            if has_status {
                status_incs.push(n);
            }
        }
    }
    let (open, resolved) = src
        .split_once("### Resolved as design point")
        .expect("Resolved-as-design-point section missing");
    let resolved_nrs = inc_numbers_in_table(resolved);
    let open_nrs = inc_numbers_in_table(open);
    for n in &status_incs {
        assert!(
            resolved_nrs.contains(n),
            "INC#{n} has a Status block in its long-form entry but is missing from the Resolved-as-design-point table"
        );
        assert!(
            !open_nrs.contains(n),
            "INC#{n} has a Status block but still appears in the Medium/Low severity tables above"
        );
    }
}

/// Every issue that appears in the PROBLEMS.md Quick-Reference as
/// still-open (not wrapped in `~~N~~`) must have a long-form
/// section below whose heading is also not crossed out.  Conversely,
/// if the long-form heading is `### ~~N~~. …` (indicating FIXED /
/// RESOLVED), the Quick-Reference row must also be crossed out or
/// carry a "Done" / "Fixed" marker.  Locks the drift between the
/// two sides of the doc called out in QUALITY.md Tier 4 item 10.
#[test]
fn problems_quickref_matches_longform_status() {
    let src = read_problems();
    let (quickref, longform) = src
        .split_once("## Interpreter Robustness")
        .expect("Interpreter Robustness section missing — PROBLEMS.md layout changed");
    let open_in_quickref = open_issue_numbers_in_quickref(quickref);
    let fixed_longform = fixed_issue_numbers_in_longform(longform);
    for n in &open_in_quickref {
        assert!(
            !fixed_longform.contains(n),
            "Issue #{n} is listed as open in PROBLEMS.md Quick-Reference but its long-form heading is crossed out (FIXED).  Update the Quick-Reference row."
        );
    }
}

/// PROBLEMS.md gained a `🔴 Currently Open (fast index)` section
/// at the top (2026-05-11) so readers can find open issues
/// without scanning the 60-row Quick-Reference table.  This guard
/// keeps the fast-index in sync with the table — every open row
/// (severity column contains `(open)`) must appear in the index,
/// and no closed row may.  When the index drifts, `make problems`
/// (which reads from the table directly) and the rendered fast
/// index disagree — bad reader experience.
#[test]
fn problems_open_index_matches_quickref() {
    let src = read_problems();
    let fast_index_start = src.find("## 🔴 Currently Open (fast index)").expect(
        "`🔴 Currently Open (fast index)` section missing — was it removed from PROBLEMS.md?",
    );
    let fast_index_end = src[fast_index_start..]
        .find("\n## Open Issues — Quick Reference")
        .expect("`Open Issues — Quick Reference` heading must follow the fast index")
        + fast_index_start;
    let fast_index = &src[fast_index_start..fast_index_end];
    let quickref_start = fast_index_end + "\n".len(); // skip the leading newline so the next `find` lands past the Quick-Reference heading itself
    // Skip past the Quick-Reference heading line so the next `\n## `
    // search lands on the FOLLOWING section (not on the Quick-Reference
    // heading itself, which would yield an empty quickref slice).
    let quickref_body_start = src[quickref_start..]
        .find('\n')
        .map_or(quickref_start, |e| quickref_start + e + 1);
    let quickref_end = src[quickref_body_start..]
        .find("\n## ")
        .map_or(src.len(), |e| e + quickref_body_start);
    let quickref = &src[quickref_body_start..quickref_end];

    // Extract IDs from the table that have `(open)` in their severity
    // column — those that MUST appear in the fast index.
    let mut open_in_table: Vec<u32> = Vec::new();
    for line in quickref.lines() {
        let l = line.trim_start();
        if !l.starts_with("| ") {
            continue;
        }
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        if cells.len() < 4 {
            continue;
        }
        let id_cell = cells[1];
        let sev_cell = cells[3];
        if !sev_cell.contains("(open)") {
            continue;
        }
        if let Ok(n) = id_cell.parse::<u32>() {
            open_in_table.push(n);
        }
    }

    // Extract IDs the fast index claims are open — `[P<n>]`
    // mentions in its rows.  Each open ID should appear once.
    let mut indexed: Vec<u32> = Vec::new();
    for line in fast_index.lines() {
        let l = line.trim_start();
        if !l.starts_with("| [P") {
            continue;
        }
        // `| [P229](...` — skip past `| [P` (4 bytes) then read digits.
        let after_p = &l[4..];
        let end = after_p
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after_p.len());
        if let Ok(n) = after_p[..end].parse::<u32>() {
            indexed.push(n);
        }
    }

    for n in &open_in_table {
        assert!(
            indexed.contains(n),
            "PROBLEMS.md Quick-Reference lists P{n} as `(open)` but the `🔴 Currently Open (fast index)` section at the top doesn't mention it.  Add a row to the fast index (and re-run `make problems` to see the canonical filtered list)."
        );
    }

    // Every fast-index entry must correspond to a real `(open)`
    // table row OR be marked `(partial)` (e.g., P229 where one
    // half closed but another open — a special case the table
    // can't express in a binary `(open)` / `(closed)` cell).
    for n in &indexed {
        let table_has_open = open_in_table.contains(n);
        let allowed_partial =
            fast_index.contains(&format!("[P{n}](#open-issues--quick-reference) (partial)"));
        assert!(
            table_has_open || allowed_partial,
            "P{n} appears in the `🔴 Currently Open (fast index)` but the Quick-Reference table doesn't mark it `(open)`.  Either close the fast-index row (the table is canonical), or annotate the index entry as `(partial)` if only part of the issue is still open."
        );
    }
}

/// The `🔴 Currently Open (fast index)` is an at-a-glance list of
/// CURRENTLY-OPEN issues only.  It must not accumulate closed-issue
/// rows or historical narration — that history lives in the canonical
/// Quick-Reference table below.  Without this guard the section
/// silently re-grows: `problems_open_index_matches_quickref` only
/// checks `| [P…]` link rows, so strikethrough rows and free prose
/// slip past it (they did, to ~285 lines, before this test landed).
///
/// Allowed shapes inside the section: the heading, a short intro
/// paragraph BEFORE the table, the table header + separator, table
/// data rows that link to the quick-ref (`| [P…]` / `| [@P…]`), blank
/// lines, and the trailing `---` thematic break.  Specifically banned:
/// strikethrough (`~~…~~`, which only ever marks a closed issue) and
/// any prose AFTER the table block starts.
#[test]
fn problems_fast_index_stays_clean() {
    let src = read_problems();
    let start = src
        .find("## 🔴 Currently Open (fast index)")
        .expect("`🔴 Currently Open (fast index)` section missing from PROBLEMS.md");
    let end = src[start..]
        .find("\n## Open Issues — Quick Reference")
        .expect("`Open Issues — Quick Reference` heading must follow the fast index")
        + start;
    let section = &src[start..end];

    // Strikethrough only ever marks a CLOSED issue → it belongs in the
    // Quick-Reference table below, never in the open-index.
    assert!(
        !section.contains("~~"),
        "`🔴 Currently Open (fast index)` contains `~~` (strikethrough) — closed issues must NOT be listed here.  Move the closed row to the Quick-Reference table below and delete it from the fast index."
    );

    let mut table_started = false;
    let mut table_ended = false;
    for line in section.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with('|') {
            assert!(
                !table_ended,
                "`🔴 Currently Open (fast index)` has a table row AFTER the table block ended: {line:?} — keep all open rows in one contiguous table."
            );
            table_started = true;
            // Header / separator rows are fine as-is.
            if t.starts_with("| # ") || t.starts_with("|---") {
                continue;
            }
            // Every data row must link to a real Quick-Reference entry.
            assert!(
                t.starts_with("| [P") || t.starts_with("| [@P"),
                "`🔴 Currently Open (fast index)` table row is not a Quick-Reference link (`| [P…]` / `| [@P…]`): {line:?}"
            );
            continue;
        }
        // A non-table, non-blank line.  Before the table it is intro
        // prose (allowed).  After the table starts, the only thing
        // permitted is the trailing `---` thematic break — anything
        // else is closed-issue narration creeping back in.
        if table_started {
            if t == "---" {
                table_ended = true;
                continue;
            }
            panic!(
                "`🔴 Currently Open (fast index)` has prose after the table block (closed-issue narration?): {line:?} — closed-issue history belongs in the Quick-Reference table / CHANGELOG_TECHNICAL.md, not the open index."
            );
        }
    }
}

/// Every caveat ID whose long-form heading is crossed out
/// (`### ~~CX~~ … DONE` / `### ~~PX~~ … DONE`) must also appear
/// crossed out in the Verification-log table at the bottom.  The
/// table is the reader's at-a-glance index — a stale row there
/// silently claims an already-finished caveat is still in flight.
#[test]
fn caveats_longform_done_matches_verification_log() {
    let src = read_caveats();
    let (body, verification) = src
        .split_once("## Verification log")
        .expect("Verification log section missing — CAVEATS.md layout changed");
    let done_ids = caveat_ids_struck_in_longform(body);
    let open_ids_in_table = caveat_ids_open_in_verification_table(verification);
    for id in &done_ids {
        assert!(
            !open_ids_in_table.contains(id),
            "Caveat '{id}' long-form heading is crossed out (DONE) but the Verification-log table still lists it as open — update the table row to '~~{id}~~ … Done'"
        );
    }
}

/// Extract caveat IDs (C7/P22/C54/…) that appear in `### ~~…~~` headings.
fn caveat_ids_struck_in_longform(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("### ~~")
            && let Some((inner, _)) = rest.split_once("~~")
        {
            // `inner` may be a compound like "C7 / P22" or "P135 / C58".
            for part in inner.split('/') {
                let id = part.trim().to_string();
                if !id.is_empty() {
                    out.push(id);
                }
            }
        }
    }
    out
}

/// Extract caveat IDs in the Verification-log table whose first cell is
/// NOT crossed out (still presented as open).  A compound cell like
/// `C58/P135` with neither side struck counts both; if the cell is
/// `~~C7/P22~~` both sides are treated as struck.
fn caveat_ids_open_in_verification_table(verification: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in verification.lines() {
        let l = line.trim_start();
        if !l.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        let first = cells[1];
        if first.is_empty() || first == "Caveat" || first.contains("---") {
            continue;
        }
        if first.starts_with("~~") {
            continue;
        }
        for part in first.split('/') {
            let id = part.trim().trim_matches('~').to_string();
            if !id.is_empty() {
                out.push(id);
            }
        }
    }
    out
}

/// Returns issue numbers in the Quick-Reference table whose first
/// cell is NOT wrapped in `~~N~~` (i.e. rows still marked open).
fn open_issue_numbers_in_quickref(quickref: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for line in quickref.lines() {
        let l = line.trim_start();
        if !l.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        let first = cells[1];
        if first.starts_with("~~") || first.is_empty() || first == "#" || first.contains('-') {
            continue;
        }
        if let Ok(n) = first.parse::<u32>() {
            out.push(n);
        }
    }
    out
}

/// Returns issue numbers whose long-form heading is crossed out
/// (`### ~~N~~. …`) — indicating FIXED / RESOLVED.
fn fixed_issue_numbers_in_longform(longform: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for line in longform.lines() {
        if let Some(rest) = line.strip_prefix("### ~~")
            && let Some(num_str) = rest.split("~~").next()
            && let Ok(n) = num_str.trim().parse::<u32>()
        {
            out.push(n);
        }
    }
    out
}

fn inc_number_from_heading(line: &str) -> Option<u32> {
    // Heading form: `## 27. ...` — number after `## ` and before `.`.
    let rest = line.strip_prefix("## ")?;
    let (num, _) = rest.split_once('.')?;
    num.trim().parse::<u32>().ok()
}

fn inc_numbers_in_table(section: &str) -> Vec<u32> {
    // Table rows look like `| 27 | ... |` — first column is the INC#.
    let mut out = Vec::new();
    for line in section.lines() {
        let l = line.trim_start();
        if !l.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        if let Ok(n) = cells[1].parse::<u32>() {
            out.push(n);
        }
    }
    out
}

/// RAII guard that restores a file's contents on drop.  The staleness
/// tests below have to let the generator overwrite the real
/// `doc/examples.js` on disk (the script hard-codes that relative
/// path), so this guard makes sure the working tree ends up exactly
/// as it was before the test ran — whether the test panicked, failed,
/// or passed.
struct FileGuard {
    path: std::path::PathBuf,
    original: Vec<u8>,
}

impl FileGuard {
    fn new(path: std::path::PathBuf) -> Self {
        let original = fs::read(&path)
            .unwrap_or_else(|e| panic!("cannot read {} for backup: {e}", path.display()));
        Self { path, original }
    }
}

impl Drop for FileGuard {
    fn drop(&mut self) {
        // Best-effort restore; panicking in Drop would obscure the
        // original test failure.
        let _ = fs::write(&self.path, &self.original);
    }
}

/// Regenerates `output_relative` by running `script` through the
/// release `loft` binary and fails with a clear remediation message
/// if the result differs from the committed version.
fn assert_generator_output_matches_committed(script: &str, output_relative: &str) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output_path = root.join(output_relative);
    let guard = FileGuard::new(output_path.clone());

    let loft_bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"));
    let out = std::process::Command::new(&loft_bin)
        .arg("--interpret")
        .arg(script)
        .current_dir(&root)
        .output()
        .unwrap_or_else(|e| panic!("spawn {} failed: {e}", loft_bin.display()));
    assert!(
        out.status.success(),
        "generator `{script}` exited non-zero: {}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let regenerated = fs::read(&output_path)
        .unwrap_or_else(|e| panic!("generator did not write {output_relative}: {e}"));
    if regenerated != guard.original {
        let mut first_diff_line = 0usize;
        let a = String::from_utf8_lossy(&regenerated);
        let b = String::from_utf8_lossy(&guard.original);
        for (i, (l, r)) in a.lines().zip(b.lines()).enumerate() {
            if l != r {
                first_diff_line = i + 1;
                break;
            }
        }
        let a_lines = a.lines().count();
        let b_lines = b.lines().count();
        panic!(
            "\n{output_relative} is stale.\n\nFix:  loft --interpret {script}\nThen: git add {output_relative} && commit\n\n\
             regenerated={a_lines} lines, committed={b_lines} lines, first diff at line {first_diff_line}.\n"
        );
    }
}

/// `doc/examples.js` is committed but auto-generated from
/// `tests/docs/*.loft` by `scripts/build-playground-examples.loft`.
/// Nothing automates the regeneration, so any change to
/// `tests/docs/*.loft` that ships without running the generator
/// leaves GitHub Pages serving stale content.  This test is the
/// guard.
///
/// The SIGSEGV that previously made this test unrunnable (P185 —
/// slot-aliasing heap corruption in the generator's
/// `for f in file("tests/docs").files()` loop) was closed by
/// plan-05 on 2026-04-23.  Un-ignored as of the same commit; the
/// generator now completes cleanly.
#[test]
fn doc_examples_js_is_up_to_date() {
    assert_generator_output_matches_committed(
        "scripts/build-playground-examples.loft",
        "doc/examples.js",
    );
}

/// `doc/gallery-examples.js` is committed but auto-generated from
/// `lib/graphics/examples/*.loft` by
/// `scripts/build-gallery-examples.loft`.  Same drift risk as
/// `doc/examples.js` — this guard catches it.
#[test]
fn doc_gallery_examples_js_is_up_to_date() {
    assert_generator_output_matches_committed(
        "scripts/build-gallery-examples.loft",
        "doc/gallery-examples.js",
    );
}

/// @PLAN12 Phase 6.12 — every fixture dir under
/// `tests/fixtures/libs/` MUST contain a `loft.toml`.  Cheap
/// structural sanity check that doesn't require network (the
/// chunk-repo drift check via `scripts/sync-fixtures.sh --check`
/// lives in a separate CI workflow because it needs to clone the
/// pinned tags).  This guard catches "someone added an empty
/// fixture dir by accident."
#[test]
fn lib_fixtures_have_loft_toml() {
    let dir = std::path::Path::new("tests/fixtures/libs");
    if !dir.exists() {
        // Pre-population state — no fixtures yet.  Acceptable.
        return;
    }
    let entries = fs::read_dir(dir).expect("read tests/fixtures/libs");
    let mut missing: Vec<String> = Vec::new();
    for ent in entries.filter_map(Result::ok) {
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }
        if !path.join("loft.toml").exists() {
            missing.push(path.display().to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "Fixture dirs without loft.toml — likely incomplete sync.  \
         Run `scripts/sync-fixtures.sh` to refresh: {missing:?}"
    );
}

/// @PLAN12 Phase 6.12 — mock-registry fixture must be valid.
/// Cheap parse-only check: if mock-registry/index.json doesn't
/// parse, every test that uses LOFT_REGISTRY_URL=file://...
/// breaks confusingly downstream.  Same for advisories.json.
#[test]
fn mock_registry_fixtures_are_valid() {
    let idx = std::path::Path::new("tests/fixtures/mock-registry/index.json");
    let adv = std::path::Path::new("tests/fixtures/mock-registry/advisories.json");
    if !idx.exists() {
        // Pre-population state.  Acceptable.
        return;
    }
    let content = fs::read_to_string(idx).expect("read mock index");
    assert!(
        content.contains("\"schema_version\""),
        "mock-registry/index.json: missing schema_version key"
    );
    if adv.exists() {
        let content = fs::read_to_string(adv).expect("read mock advisories");
        assert!(
            content.contains("\"advisories\""),
            "mock-registry/advisories.json: missing advisories key"
        );
    }
}

/// T3.2 — the README invites the reader to "read all N lines" of Brick Buster,
/// and that number is the hook: it is the proof that the whole game really is
/// one readable file.  A hardcoded count rots the moment the game is edited, so
/// pin it — a stale number in the most-read paragraph of the README is exactly
/// the kind of small lie the link checker exists to prevent elsewhere.
#[test]
fn readme_brick_buster_line_count_is_current() {
    let game = "tools/brick-buster/25-brick-buster.loft";
    let actual = fs::read_to_string(game)
        .unwrap_or_else(|e| panic!("{game}: {e}"))
        .lines()
        .count();
    let readme = fs::read_to_string("README.md").expect("README.md");
    // The claim is written with a thousands separator: "read all 1,983 lines".
    let claimed = readme
        .split("read all ")
        .nth(1)
        .and_then(|rest| rest.split(" lines").next())
        .map(|n| n.replace(',', ""))
        .and_then(|n| n.parse::<usize>().ok())
        .expect("README must state 'read all <N> lines' for the one-file claim");
    assert_eq!(
        claimed, actual,
        "README says Brick Buster is {claimed} lines but {game} has {actual}. \
         Update the number in README.md (it is the proof behind the one-file claim)."
    );
}

/// @PLN146 F4 — the Makefile and the manifest name ONE atlas packer, not two.
///
/// `tools/brick-buster/loft.toml` declares the packer as a `[[build.asset]]` `run`,
/// which is what `loft build` / `loft check` execute.  `make game` and `make play`
/// cannot go through `loft build` — it has no assets-only mode and would
/// compile-check the whole game to reach the asset phase — so the Makefile names the
/// script itself.  That is two hand-written spellings of one fact, and the failure
/// when they drift is quiet: the Makefile keeps building whatever the OLD path points
/// at while the manifest describes something else, so `make game` ships a stale pack
/// and `loft build` a fresh one.
///
/// Pin them: the `run` the manifest declares must be the path the Makefile invokes,
/// and the `outputs` it promises must be what the Makefile checks for.
#[test]
fn brick_buster_pack_step_matches_its_manifest() {
    let toml = fs::read_to_string("tools/brick-buster/loft.toml").expect("brick-buster loft.toml");
    let run = toml
        .lines()
        .find_map(|l| l.trim().strip_prefix("run"))
        .and_then(|r| r.split('"').nth(1).map(str::to_string))
        .expect("loft.toml must declare a [[build.asset]] `run`");
    let makefile = fs::read_to_string("Makefile").expect("Makefile");
    let invoked = format!("tools/brick-buster/{run}");
    assert!(
        makefile.contains(&invoked),
        "loft.toml's [[build.asset]] runs `{run}`, but the Makefile never invokes \
         `{invoked}` — the two spellings of the atlas packer have drifted."
    );
    // Every declared output must be something the Makefile can see is absent.
    for out in toml
        .lines()
        .find_map(|l| l.trim().strip_prefix("outputs"))
        .and_then(|o| o.split('[').nth(1))
        .and_then(|o| o.split(']').next())
        .expect("loft.toml must declare [[build.asset]] `outputs`")
        .split(',')
        .filter_map(|o| o.split('"').nth(1))
    {
        assert!(
            makefile.contains(&format!("tools/brick-buster/{out}")) || out.ends_with(".meta.store"),
            "loft.toml promises `{out}` but nothing in the Makefile checks for it."
        );
    }
}

/// @PLN78 step 6 — the installer's target triples must be ones a release publishes.
///
/// `scripts/install.sh` derives the artifact name from `uname`, and
/// `self_update::host_triple` derives the registry lookup key the same way.  They are
/// two hand-written derivations of one fact, in different languages, and the failure
/// when they drift is silent: the installer 404s, or `self-update` reports "published,
/// but not built for your platform" about the artifact meant for that host.  That
/// already happened once — Linux composed `-gnu` while releases ship `-musl`.
///
/// So pin them together: every triple the shell can produce must appear in
/// `PUBLISHED_TRIPLES`, which is itself checked against a real release.
/// Gated on `registry`, the feature that compiles `self_update` at all: without
/// it the list this pins against does not exist, and the local `--all-targets
/// --no-default-features` clippy the docs prescribe failed to compile. CI never
/// caught it because its own no-default-features step is `cargo check` WITHOUT
/// `--all-targets`, so no test target was ever built that way.
#[cfg(feature = "registry")]
#[test]
fn installer_and_self_update_agree_on_the_published_triples() {
    let sh = std::fs::read_to_string("scripts/install.sh").expect("read install.sh");
    let published = loft::self_update::PUBLISHED_TRIPLES;
    // The shell composes `$arch-<rest>`; check each literal suffix it can emit.
    for suffix in ["-apple-darwin", "-unknown-linux-musl", "-pc-windows-msvc"] {
        if !sh.contains(suffix) {
            continue;
        }
        assert!(
            published.iter().any(|t| t.ends_with(suffix)),
            "install.sh can produce a `{suffix}` triple that no release publishes"
        );
    }
    for arch in ["x86_64", "aarch64"] {
        assert!(
            sh.contains(arch),
            "install.sh must map uname's output to `{arch}`, which releases publish"
        );
    }
    // And the reverse: a published UNIX target the installer cannot name is a target
    // nobody can install with it.
    for t in published {
        if t.contains("windows") {
            continue; // the installer sends Windows users to the releases page
        }
        let suffix = t.split_once('-').expect("triple has an arch prefix").1;
        assert!(
            sh.contains(suffix),
            "release publishes `{t}` but install.sh cannot name it"
        );
    }
}

/// @PLN78 step 7 — the release RECORDS the build stamp the verifier has to reproduce.
///
/// `build.rs` bakes `LOFT_BUILD_ID` from `git rev-parse`, falling back to a TIMESTAMP.  A
/// release is cut from a git checkout, so it baked a commit; `repro-verify.sh` rebuilds from
/// the release's source ARCHIVE, which has no `.git`, so it fell through to
/// seconds-since-epoch — a different string, of a different length, changing on every run.
/// Every published target failed to reproduce, and the mismatch read as a source problem
/// rather than as a stamp the build had invented.
///
/// The contract now spans three files and drifts silently if any one moves: the writer
/// records `commit`, the verifier reads it back, and `build.rs` honours the override.  This
/// pins all three ends — a rename in one file is caught here rather than by a weekly job
/// nobody blames on the rename.
#[test]
fn the_release_records_the_build_id_the_verifier_replays() {
    let mk = std::fs::read_to_string("scripts/make-release.sh").expect("read make-release.sh");
    let vf = std::fs::read_to_string("scripts/repro-verify.sh").expect("read repro-verify.sh");
    let br = std::fs::read_to_string("build.rs").expect("read build.rs");

    assert!(
        mk.contains("echo \"commit = "),
        "make-release.sh must record the build commit in BUILD-INFO — without it the \
         verifier cannot reproduce LOFT_BUILD_ID and every target fails"
    );
    assert!(
        vf.contains("^commit = "),
        "repro-verify.sh must read `commit` back out of BUILD-INFO"
    );
    assert!(
        vf.contains("LOFT_BUILD_ID=\"$BUILD_COMMIT\""),
        "repro-verify.sh must hand the recorded commit to the rebuild as LOFT_BUILD_ID"
    );
    assert!(
        br.contains("LOFT_BUILD_ID") && br.contains("rerun-if-env-changed=LOFT_BUILD_ID"),
        "build.rs must honour an explicit LOFT_BUILD_ID — and re-run when it changes, or a \
         second verification reuses the first one's baked id"
    );
}

/// @PLN78 step 7 — every triple we PUBLISH must also be rebuilt from source.
///
/// The two lists drift in one direction that is silent: adding a target to `release.yml`
/// ships a binary, and forgetting the matching `repro-build.yml` row means nobody ever
/// rebuilds it — the verification gap looks exactly like full coverage from the outside.
/// Gated on `registry` for the same reason as
/// `installer_and_self_update_agree_on_the_published_triples`: `PUBLISHED_TRIPLES`
/// is compiled out without it.
#[cfg(feature = "registry")]
#[test]
fn repro_matrix_covers_every_published_triple() {
    let wf =
        std::fs::read_to_string(".github/workflows/repro-build.yml").expect("read repro-build.yml");
    for triple in loft::self_update::PUBLISHED_TRIPLES {
        assert!(
            wf.contains(triple),
            "`{triple}` is published but never rebuilt: add a matrix row to \
             .github/workflows/repro-build.yml (on the runner release.yml builds it on)"
        );
    }

    // And each row's RUNNER must be the one the RELEASE builds that target on. Coverage
    // alone let a row pass while proving nothing: `x86_64-apple-darwin` sat on `macos-14`
    // (arm64) while the release built it natively on `macos-13` (x86_64), so the job
    // rebuilt aarch64 and reported it under the x86_64 name — x86_64 was never verified
    // while looking verified. A rebuild proves reproducibility only when it repeats the
    // release's own build: same runner architecture, hence the same toolchain and the
    // same cross-or-native decision. Since GitHub retired its Intel macOS image the
    // release CROSS-builds `x86_64-apple-darwin` on Apple Silicon, and the rebuild has to
    // do exactly that too — so the check compares the two workflows' runners for each
    // target rather than the runner against the target's own architecture, which no
    // longer has an image to satisfy it.
    //
    // Keyed on the runner image's architecture, which is the fact that went wrong:
    // `macos-13` was the x86_64 image, `macos-14`/`macos-15`/`macos-latest` are arm64.
    let arch_of_runner = |os: &str| -> &'static str {
        match os {
            "macos-13" => "x86_64",
            "macos-14" | "macos-15" | "macos-latest" => "aarch64",
            o if o.starts_with("ubuntu") || o.starts_with("windows") => "x86_64",
            _ => panic!(
                "runner `{os}`, whose architecture this check does not know — add it to \
                 `arch_of_runner` so the pairing stays checkable"
            ),
        }
    };
    // `(target, os)` per matrix item, whichever key an item lists first: release.yml
    // writes `- target:` then `os:`, repro-build.yml writes `- os:` then `target:`.
    let matrix_pairs = |yaml: &str| -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        let (mut target, mut os): (Option<String>, Option<String>) = (None, None);
        for line in yaml.lines() {
            let t = line.trim();
            if t.starts_with("- ") {
                target = None;
                os = None;
            }
            let t = t.trim_start_matches("- ").trim();
            if let Some(v) = t.strip_prefix("target:") {
                target = Some(v.trim().to_string());
            } else if let Some(v) = t.strip_prefix("os:") {
                os = Some(v.trim().to_string());
            }
            if let (Some(tg), Some(o)) = (&target, &os) {
                pairs.push((tg.clone(), o.clone()));
                target = None;
                os = None;
            }
        }
        pairs
    };
    let release =
        std::fs::read_to_string(".github/workflows/release.yml").expect("read release.yml");
    let built_on = matrix_pairs(&release);
    for (target, os) in matrix_pairs(&wf) {
        if !loft::self_update::PUBLISHED_TRIPLES.contains(&target.as_str()) {
            continue;
        }
        let Some((_, release_os)) = built_on.iter().find(|(t, _)| *t == target) else {
            panic!(
                "release.yml has no build row for `{target}`, so nothing says where to rebuild it"
            );
        };
        assert_eq!(
            arch_of_runner(&os),
            arch_of_runner(release_os),
            "repro-build.yml rebuilds `{target}` on `{os}` but release.yml builds it on \
             `{release_os}`: a rebuild on another architecture is a different build, so it \
             could only ever report a mismatch. Give it a runner of the release's architecture"
        );
    }
}

/// No source file may contain a byte that makes `grep` treat it as BINARY, because a
/// binary file is skipped SILENTLY — no match, no warning, no non-zero exit.
///
/// Measured 2026-08-22: one stray NUL, pasted into a comment in
/// `src/database/format.rs` that was quoting the one-character string a bug produced,
/// made `grep`/`ripgrep` classify all 99 KB of it as data. Every `grep -rn` over `src/`
/// had been missing that file — including the sweep that was at that moment auditing
/// type-driven `match` arms, which is how it surfaced: the backtrace named a function
/// (`ShowDb::has_visible_field`) that `grep -rn` insisted did not exist anywhere.
///
/// That is worth a gate rather than a fix, because the failure mode is invisible in both
/// directions: the file looks fine in an editor, and the search that misses it reports
/// success. loft's development model runs on grepping this tree (CLAUDE.md § Key
/// commands, `scripts/idx`), so a file the tools cannot see is a hole in the method, not
/// just in one search.
#[test]
fn no_source_file_is_invisible_to_grep() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p
                .extension()
                .is_some_and(|x| x == "rs" || x == "loft" || x == "md")
            {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    for root in ["src", "doc/claude", "tests/scripts", "default"] {
        walk(std::path::Path::new(root), &mut files);
    }
    assert!(
        files.len() > 500,
        "the walk found only {} files — it is not looking where it thinks",
        files.len()
    );

    let mut bad = Vec::new();
    for p in &files {
        let Ok(bytes) = fs::read(p) else { continue };
        // A NUL is what actually trips grep's binary heuristic; report the offset and
        // line so the fix is a one-character edit rather than a hunt.
        if let Some(off) = bytes.iter().position(|b| *b == 0) {
            let line = bytes[..off].iter().filter(|b| **b == b'\n').count() + 1;
            bad.push(format!("{}:{line} (byte {off})", p.display()));
        }
    }
    assert!(
        bad.is_empty(),
        "these files contain a NUL byte, so grep skips them silently:\n  {}\n\
         Replace the stray byte — a comment quoting a NUL should describe it in words.",
        bad.join("\n  ")
    );
}

/// Every `@FR-<Rule>` citation resolves to a rule `doc/claude/formal/` actually defines, and
/// no rule is defined twice.
///
/// The citations are what make "which sites enforce this rule?" a lookup instead of an
/// archaeology (CLAUDE.md § Tracker tags; `formal/README.md` § Rule tags).  That only holds
/// while they resolve, and a citation rots silently: the rule gets renamed, or its entry is
/// edited away, and the comment still reads correctly.  Both failure modes were REAL before
/// this gate existed — `L-Ref` was two different rules in two docs, and `D-bind-11`'s entry
/// had been deleted by an edit whose slice anchor sat inside it while its register line still
/// said `OPEN: 1`.  Nothing else noticed either; a failed citation is what surfaced them.
///
/// `scripts/rule_tags.py check` is the same command a person runs by hand, so the gate and
/// the tool cannot drift.  Skipped (not failed) where `python3` is unavailable — this is a
/// consistency check, not a capability the build depends on.
#[test]
fn every_rule_citation_resolves() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = match std::process::Command::new("python3")
        .arg(root.join("scripts/rule_tags.py"))
        .arg("check")
        .current_dir(root)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("SKIP every_rule_citation_resolves: python3 unavailable ({e})");
            return;
        }
    };
    assert!(
        out.status.success(),
        "rule-tag citations are inconsistent:\n{}{}\n\
         Run `python3 scripts/rule_tags.py check` to reproduce; \
         `list` shows every defined rule and `sites <tag>` the citations of one.",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// A diagnostic spells a type the way its reader could have written it.
///
/// `Type::name` is the SCHEMA KEY — `typedef.rs` builds wrapper types from it and `state`
/// looks stores up by it — so it renders a keyed collection in the notation the compiler
/// identifies it by: `index<Rec,[("id", true)]>`, carrying a Rust tuple and a boolean whose
/// meaning (ascending) has no spelling in the language, where the author wrote
/// `index<Rec[id]>`. `Type::source_name` answers the other question. A refusal that names
/// the first one sends its reader looking for a type that is not in their program.
///
/// Nothing FAILS when a message is merely unreadable: the compiler is right, the program is
/// wrong, and only the author pays — so no gate moves and no test goes red. That is why the
/// drift returned three times (loft#956, loft#1434, loft#1445), each pass converting the
/// sites someone happened to have a symptom for and leaving a residue for the next one. This
/// gate is the structural answer: forgetting is a red check rather than a fourth residue.
///
/// Shells out to the same command a person runs, so the gate and the tool cannot drift.
/// Skipped (not failed) where `python3` is unavailable — a consistency check, not a
/// capability the build depends on.
#[test]
fn diagnostics_spell_types_as_the_author_wrote_them() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = match std::process::Command::new("python3")
        .arg(root.join("scripts/diagnostic_spelling.py"))
        .arg("check")
        .current_dir(root)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!(
                "SKIP diagnostics_spell_types_as_the_author_wrote_them: python3 unavailable ({e})"
            );
            return;
        }
    };
    assert!(
        out.status.success(),
        "a diagnostic renders a type with the schema key:\n{}{}\n\
         Run `python3 scripts/diagnostic_spelling.py check` to reproduce; \
         `list` shows every render the gate governs.",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// `src/ir_schema_gen.rs` must be what `tools/ir_schema/ir.loft` generates.
///
/// The generated file IS the store layout — record sizes, field byte offsets, the `Node`
/// discriminants that `data_store.rs` mirrors in its baked `DISC_*` constants — and it
/// carries a `DO NOT EDIT — regenerate` header that nothing enforced. It drifted: `Key`
/// gained a `start` field in loft#812, the generated file was updated to match, `ir.loft`
/// — the source — was not, and the next regeneration silently dropped the field and took
/// `KEY_STRIDE` from 24 to 16. A wrong layout is not a build error; it is a wrong byte
/// offset, found later and elsewhere.
///
/// Shells out to the same script a person runs, so the gate and the tool cannot drift.
/// The script SKIPS (exit 0, with a line saying so) when there is no built `loft` to
/// regenerate with, or no `python3`; that line is surfaced here, because a check that
/// quietly did not run must not read as a check that passed.
#[test]
fn ir_schema_gen_matches_its_loft_source() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = match std::process::Command::new(root.join("scripts/ir_schema_check.sh"))
        .current_dir(root)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("SKIP ir_schema_gen_matches_its_loft_source: cannot run the script ({e})");
            return;
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.starts_with("SKIP ") {
        eprintln!(
            "SKIP ir_schema_gen_matches_its_loft_source: {}",
            stdout.trim()
        );
        return;
    }
    assert!(
        out.status.success(),
        "the generated IR schema does not match its source:\n{}{}",
        stdout,
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Every site that FREES on the empty-`deps` proxy must consult the never-free override.
///
/// `formal/ownership.md` @FR-O-Proxy. An empty dep list does not mean "owner" — it means
/// "nothing recorded a dep here", which is also true of a borrow nobody populated, so
/// freeing on it alone releases a store someone else owns (loft#723).
///
/// Shells out to the same script a person runs, so the gate and the tool cannot drift.
#[test]
fn o_proxy_frees_consult_the_override() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = match std::process::Command::new("python3")
        .arg(root.join("scripts/o_proxy_check.py"))
        .current_dir(root)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("SKIP o_proxy_frees_consult_the_override: python3 unavailable ({e})");
            return;
        }
    };
    assert!(
        out.status.success(),
        "@FR-O-Proxy violated:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// @PLN121 step 2 — a doc page that RUNS and asserts nothing proves nothing.
///
/// `tests/docs/*.loft` are the pages `gendoc` renders, and their whole claim is that
/// the doc cannot disagree with the code because the doc is generated from executed
/// source.  Execution alone does not carry that claim: a page that calls its subject
/// and checks no result has demonstrated that the call does not crash, and nothing
/// else — while rendering identically to a page that proves its output.  That is the
/// worst shape for a mechanism whose entire value is trustworthiness, and it is
/// invisible from the rendered page.
///
/// Measured when this landed: 33 pages, one hole — `16-parser.loft` called
/// `parser::parse` five times and asserted nothing.
///
/// **Counted over the whole FILE, not over `fn main`.** Two pages delegate from `main`
/// to `test_*` / `show_*` helpers that do the asserting (17 and 28 asserts), and a
/// `main`-only rule reports both as holes — a lint whose false positives are the
/// idiomatic shape gets silenced rather than fixed.  A page is a hole only when
/// NOTHING in it asserts.
#[test]
fn every_doc_page_asserts_something() {
    let mut silent = Vec::new();
    let dir = std::path::Path::new("tests/docs");
    let mut pages = 0;
    for entry in std::fs::read_dir(dir).expect("tests/docs is readable") {
        let path = entry.expect("dir entry").path();
        if !path.is_file() || path.extension().is_none_or(|e| e != "loft") {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("page is readable");
        // An `@IGNORE`d page is not run, so it cannot be asked to prove anything.
        if body.contains("@IGNORE") {
            continue;
        }
        pages += 1;
        // `assert` covers the plain form and `Test::advice()`-style helpers alike;
        // `@EXPECT_ERROR` is a page whose CLAIM is the diagnostic, which the harness
        // checks for it.
        if !body.contains("assert") && !body.contains("@EXPECT_ERROR") {
            silent.push(path.file_name().unwrap().to_string_lossy().to_string());
        }
    }
    assert!(pages >= 30, "expected the doc corpus, found {pages} pages");
    assert!(
        silent.is_empty(),
        "these doc pages run and assert nothing, so they prove only that the code does \
         not crash — and they render exactly like a page that proves its output:\n  {}\n\
         Add an assert on the result each example produces, or mark the page \
         @EXPECT_ERROR if the diagnostic IS its claim.",
        silent.join("\n  ")
    );
}

/// The two `try_swap` dispatch tables in `parser/operators.rs` must stay identical.
///
/// `rewrite_outer_arith_to_nullable` and `rewrite_subtree_to_nullable` each carry their own
/// copy of the base-op → Nullable-peer table, and the second one says why in its own comment:
/// *"kept inline (no shared helper) so the dispatch table stays grep-discoverable from both
/// swap sites."*  That is a deliberate trade, and this test is what pays for it — a duplicate
/// kept for discoverability still has to be kept in agreement by something other than memory.
///
/// It is worth a gate because the sibling list in this family DID drift: the wrapper-getter
/// list to descend through had a third copy in `parser/control.rs`, @P356 extended one copy
/// past the integer wrappers, and the other kept the four it was born with — so a defended
/// read of a non-integer vector reported while `vector<integer>` stayed silent, on both
/// backends.  That copy is now gone (the defended-fault-site pass calls the canonical
/// function), which leaves these two tables as the remaining duplication.
#[test]
fn the_nullable_swap_tables_do_not_drift() {
    let src = std::fs::read_to_string("src/parser/operators.rs").expect("read operators.rs");
    let mut tables: Vec<Vec<String>> = Vec::new();
    for (idx, _) in src.match_indices("fn try_swap(") {
        let rest = &src[idx..];
        let end = rest
            .find("_ => return false,")
            .expect("try_swap body has no default arm");
        let mut arms: Vec<String> = Vec::new();
        for line in rest[..end].lines() {
            let t = line.trim();
            if t.starts_with('"') && t.contains("=>") {
                arms.push(t.trim_end_matches(',').to_string());
            }
        }
        tables.push(arms);
    }
    assert_eq!(
        tables.len(),
        2,
        "expected exactly two try_swap tables in operators.rs; found {}. \
         If one was added or removed, update this gate deliberately.",
        tables.len()
    );
    assert!(
        !tables[0].is_empty(),
        "extracted an EMPTY table — the gate would pass vacuously; the arm shape must have changed"
    );
    assert_eq!(
        tables[0], tables[1],
        "the two Nullable swap tables have drifted; they are duplicated on purpose for \
         grep-discoverability, so they must agree entry for entry"
    );
}

/// The `unspan` backlog table in QUALITY.md must match what the audit actually reports.
///
/// That table is a count offered as OPEN WORK, and the doc says in as many words that such a
/// count "has to be right, or it is a bill someone else pays in review". It went stale inside
/// the same session that wrote the rule: fixes moved the number from 13 to 10 and the table
/// kept saying 13. A hand-maintained figure describing a mechanically-derivable fact is a
/// promise to remember, and this is the check that replaces remembering.
///
/// Skips rather than fails when the script cannot run, so a machine without python3 does not
/// turn a doc gate into a red build.
#[test]
fn quality_unspan_table_matches_the_audit() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = match std::process::Command::new("python3")
        .arg(root.join("scripts/ir_walker_audit.py"))
        .arg("unspan")
        .current_dir(root)
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => {
            eprintln!("SKIP quality_unspan_table_matches_the_audit: cannot run the audit");
            return;
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let nums: Vec<u32> = stdout
        .lines()
        .filter_map(|l| l.rsplit(':').next())
        .filter_map(|t| t.trim().parse::<u32>().ok())
        .take(3)
        .collect();
    assert_eq!(nums.len(), 3, "audit header changed shape; got: {stdout}");

    // Anchored on the table's own header, not on "the first line that looks like a numeric
    // row": another numeric table added above it would otherwise silently become the subject
    // of this gate, which is the failure a doc check must not have.
    let doc = std::fs::read_to_string(root.join("doc/claude/QUALITY.md")).expect("QUALITY.md");
    let lines: Vec<&str> = doc.lines().collect();
    let header = lines
        .iter()
        .position(|l| l.starts_with("| sites discriminating on 2+ specific"))
        .expect("the unspan table header is gone from QUALITY.md — update this gate with it");
    let row = lines
        .get(header + 2)
        .expect("the unspan table is truncated after its header");
    let cells: Vec<u32> = row
        .split('|')
        .filter_map(|c| c.trim().trim_matches('*').parse::<u32>().ok())
        .collect();
    assert_eq!(
        cells, nums,
        "QUALITY.md's unspan row says {cells:?} and `ir_walker_audit.py unspan` reports \
         {nums:?} — re-run the audit and update the row"
    );
}

/// The `spellings` table in QUALITY.md must match what the audit actually reports.
///
/// The sibling of [`quality_unspan_table_matches_the_audit`], for the same reason: the row is
/// a count offered as open work, and a hand-maintained figure describing a mechanically
/// derivable fact is a promise to remember. This is the check that replaces remembering.
///
/// Skips rather than fails when the script cannot run, so a machine without python3 does not
/// turn a doc gate into a red build.
#[test]
fn quality_spellings_table_matches_the_audit() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = match std::process::Command::new("python3")
        .arg(root.join("scripts/ir_walker_audit.py"))
        .arg("spellings")
        .current_dir(root)
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => {
            eprintln!("SKIP quality_spellings_table_matches_the_audit: cannot run the audit");
            return;
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let nums: Vec<u32> = stdout
        .lines()
        .filter_map(|l| l.rsplit(':').next())
        .filter_map(|t| t.trim().parse::<u32>().ok())
        .take(3)
        .collect();
    assert_eq!(nums.len(), 3, "audit header changed shape; got: {stdout}");

    // Anchored on the table's own header, like the unspan gate: another numeric table added
    // above it must not silently become the subject of this check.
    let doc = std::fs::read_to_string(root.join("doc/claude/QUALITY.md")).expect("QUALITY.md");
    let lines: Vec<&str> = doc.lines().collect();
    let header = lines
        .iter()
        .position(|l| l.starts_with("| functions resolving a projection by OP NAME"))
        .expect("the spellings table header is gone from QUALITY.md — update this gate with it");
    let row = lines
        .get(header + 2)
        .expect("the spellings table is truncated after its header");
    let cells: Vec<u32> = row
        .split('|')
        .filter_map(|c| c.trim().trim_matches('*').parse::<u32>().ok())
        .collect();
    assert_eq!(
        cells, nums,
        "QUALITY.md's spellings row says {cells:?} and `ir_walker_audit.py spellings` reports \
         {nums:?} — re-run the audit and update the row"
    );
}

#[test]
fn quality_optional_table_matches_the_audit() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = match std::process::Command::new("python3")
        .arg(root.join("scripts/ir_walker_audit.py"))
        .arg("optional")
        .current_dir(root)
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => {
            eprintln!("SKIP quality_optional_table_matches_the_audit: cannot run the audit");
            return;
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let nums: Vec<u32> = stdout
        .lines()
        .filter_map(|l| l.rsplit(':').next())
        .filter_map(|t| t.trim().parse::<u32>().ok())
        .take(4)
        .collect();
    assert_eq!(nums.len(), 4, "audit header changed shape; got: {stdout}");

    // Anchored on the table's own header, like the unspan gate: another numeric table added
    // above it must not silently become the subject of this check.
    let doc = std::fs::read_to_string(root.join("doc/claude/QUALITY.md")).expect("QUALITY.md");
    let lines: Vec<&str> = doc.lines().collect();
    let header = lines
        .iter()
        .position(|l| l.starts_with("| functions discriminating on a `Type` variant"))
        .expect("the optional table header is gone from QUALITY.md — update this gate with it");
    let row = lines
        .get(header + 2)
        .expect("the optional table is truncated after its header");
    let cells: Vec<u32> = row
        .split('|')
        .filter_map(|c| c.trim().trim_matches('*').parse::<u32>().ok())
        .collect();
    assert_eq!(
        cells, nums,
        "QUALITY.md's optional row says {cells:?} and `ir_walker_audit.py optional` reports \
         {nums:?} — re-run the audit and update the row"
    );
}

/// A topic's metadata must not reach the reader as prose.
///
/// `gather_topics` scans a whole chapter for `// @NAME:` and `// @TITLE:`, so those two
/// lines are consumed as directives wherever they sit — and the renderer used to drop
/// them only while it believed it was still in the header block. That block ends at the
/// first line the header vocabulary does not recognise, so the Feature catalogue's
/// generated `// GENERATED by … DO NOT EDIT.` note ended it on line 3 and pushed both
/// directives into the opening paragraph of the published page, `print.html` and the
/// Typst source for the PDF — all four release bundles, with the page's own title read
/// back to the reader and a maintainer's instruction addressed to someone who cannot
/// act on it.
///
/// The `<pre id="lp-source">` block at the foot of a page is the chapter's raw source,
/// fed to the run panel, so it carries the header on purpose and is not in scope.
#[test]
fn no_topic_renders_its_own_directives_as_prose() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let leaks = ["@NAME:", "@TITLE:", "DO NOT EDIT"];

    // The subject is the TOPIC pages — one per `tests/docs/*.loft`. A library's
    // `-src.html` is a syntax-highlighted source listing, where a chapter's own
    // `// @NAME:` line is the content and rendering it is the point.
    let mut offenders: Vec<String> = Vec::new();
    let mut pages_read = 0_usize;
    for entry in std::fs::read_dir(root.join("tests/docs")).expect("tests/docs is unreadable") {
        let topic = entry.expect("tests/docs entry").path();
        if topic.extension().is_none_or(|e| e != "loft") {
            continue;
        }
        let stem = topic
            .file_stem()
            .expect("stem")
            .to_string_lossy()
            .into_owned();
        let path = root.join("doc").join(format!("{stem}.html"));
        // `00-general` is the front matter of the index rather than a page of its own.
        let Ok(html) = std::fs::read_to_string(&path) else {
            continue;
        };
        let start = html.find("<article>").expect("a topic page has an article");
        // `<section id="loft-panel">` holds the run panel, whose `<pre id="lp-source">`
        // is the chapter's raw source and carries the header on purpose.
        let end = ["<section id=\"loft-panel\"", "</article>"]
            .iter()
            .filter_map(|m| html[start..].find(m))
            .min()
            .map_or(html.len(), |e| start + e);
        pages_read += 1;
        let body = &html[start..end];
        for probe in leaks {
            if body.contains(probe) {
                offenders.push(format!("{}: {probe}", path.display()));
            }
        }
    }

    // The print bundle and the PDF source have no run panel, so they are checked whole.
    for name in ["print.html", "loft-reference.typ"] {
        let text = std::fs::read_to_string(root.join("doc").join(name)).expect("bundle");
        for probe in leaks {
            if text.contains(probe) {
                offenders.push(format!("doc/{name}: {probe}"));
            }
        }
    }

    assert!(
        pages_read > 30,
        "only {pages_read} topic pages had an <article> — the check lost its subject"
    );
    assert!(
        offenders.is_empty(),
        "a topic's own metadata is rendered as prose in the release bundles: {offenders:#?}"
    );
}

/// Read every `<div class="item">` on the published Standard Library pages as
/// (signature, documentation).
fn stdlib_entries() -> Vec<(String, String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root.join("doc")).expect("doc/ is unreadable") {
        let path = entry.expect("doc entry").path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if !name.starts_with("stdlib") || !name.ends_with(".html") {
            continue;
        }
        let html = std::fs::read_to_string(&path).expect("stdlib page");
        let Some(start) = html.find("<article>") else {
            continue;
        };
        let end = html[start..]
            .find("</article>")
            .map_or(html.len(), |e| start + e);
        let body = &html[start..end];
        let mut rest = body;
        while let Some(i) = rest.find("<div class=\"item\">") {
            rest = &rest[i..];
            let Some(open) = rest.find("<pre><code>") else {
                break;
            };
            let Some(close) = rest[open..].find("</code></pre>") else {
                break;
            };
            let sig = strip_tags(&rest[open + 11..open + close]);
            let after = &rest[open + close..];
            let doc_end = after.find("</div>").unwrap_or(after.len());
            out.push((name.clone(), sig, strip_tags(&after[..doc_end])));
            rest = &after[doc_end..];
        }
    }
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every `pub fn` the standard library declares reaches the published Standard Library.
///
/// `gendoc` used to read a hard-coded three-element list of `default/*.loft`, so
/// `04_stacktrace`, `05_coroutine`, `06_json` and `07_reflect` — added later — contributed
/// nothing: the whole JSON and reflection API was absent from the reference while the JSON
/// chapter and @F42 documented it. A list of filenames reads as a decision and was a sample.
#[test]
fn every_public_stdlib_function_is_published() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut declared: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(root.join("default")).expect("default/ is unreadable") {
        let path = entry.expect("default entry").path();
        if path.extension().is_none_or(|e| e != "loft") {
            continue;
        }
        for line in std::fs::read_to_string(&path).expect("stdlib file").lines() {
            let t = line.trim();
            if t.starts_with("pub fn ") {
                declared.push(
                    t.trim_end_matches(['{', ';'])
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
        }
    }
    assert!(
        declared.len() > 150,
        "only {} `pub fn` found in default/ — the scan lost its subject",
        declared.len()
    );

    let rendered: std::collections::HashSet<String> = stdlib_entries()
        .into_iter()
        .map(|(_, sig, _)| sig.trim_end_matches(['{', ';']).trim().to_string())
        .collect();
    let missing: Vec<&String> = declared.iter().filter(|d| !rendered.contains(*d)).collect();
    assert!(
        missing.is_empty(),
        "{} public stdlib function(s) are declared but reach no page of the published \
         Standard Library — run `cargo run --bin gendoc`, and check that every \
         `default/*.loft` is read:\n  {}",
        missing.len(),
        missing
            .iter()
            .map(|m| m.as_str())
            .take(12)
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// Every published Standard Library entry says what it is for.
///
/// A blank line between a doc comment and its declaration used to orphan the doc, so 43
/// entries reached the reader as a bare signature — `sin`, `sqrt`, `floor`, `round`,
/// `split` among them, while the same authoring shape without the blank line rendered fine.
#[test]
fn every_published_stdlib_entry_carries_its_documentation() {
    let bare: Vec<String> = stdlib_entries()
        .into_iter()
        .filter(|(_, _, doc)| doc.is_empty())
        .map(|(page, sig, _)| format!("{page}: {}", sig.lines().next().unwrap_or("")))
        .collect();
    assert!(
        bare.is_empty(),
        "{} Standard Library entries are published with no documentation at all:\n  {}",
        bare.len(),
        bare.join("\n  ")
    );
}

/// A Standard Library section is named and described for its reader.
///
/// Its name becomes the page title and the URL, and its description is the first thing on
/// the page. Both drew from the maintainer's side of `default/*.loft`: sections shipped as
/// "System directories (#635)" and "Vector operations (T2-8, T2-5)", and the Text section's
/// description was `OpVarText`'s comment — "Read the value of a variable and put a reference
/// to it on the stack".
#[test]
fn no_stdlib_section_shows_the_reader_a_tracker_tag_or_a_private_name() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    // The private declarations are derived, not listed: anything `default/` declares
    // without `pub` is a name the reader has no way to use.
    let mut private: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(root.join("default")).expect("default/") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "loft") {
            continue;
        }
        for line in std::fs::read_to_string(&path).expect("stdlib file").lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("fn ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if name.len() > 3 {
                    private.push(name);
                }
            }
        }
    }
    assert!(
        private.len() > 20,
        "no private stdlib declarations found — the scan broke"
    );

    let mut bad: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(root.join("doc")).expect("doc/") {
        let path = entry.expect("doc entry").path();
        let file = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if !file.starts_with("stdlib-") || !file.ends_with(".html") {
            continue;
        }
        // A tracker tag in the FILENAME is a tracker tag in the published URL.
        if file.contains("pln") || file.contains("-635") || file.contains("t2-8") {
            bad.push(format!("{file}: the page URL carries a tracker tag"));
        }
        let html = std::fs::read_to_string(&path).expect("page");
        let Some(start) = html.find("<article>") else {
            continue;
        };
        let end = html[start..]
            .find("</article>")
            .map_or(html.len(), |e| start + e);
        let mut rest = &html[start..end];
        while let Some(i) = rest.find("<div class=\"section-desc\">") {
            rest = &rest[i..];
            let stop = rest.find("</div>").unwrap_or(rest.len());
            let desc = strip_tags(&rest[..stop]);
            for name in &private {
                if desc.contains(name.as_str()) {
                    bad.push(format!(
                        "{file}: the section description names `{name}`, \
                                      which is private: {desc:.90}"
                    ));
                }
            }
            rest = &rest[stop..];
        }
    }
    assert!(
        bad.is_empty(),
        "the published Standard Library shows the reader the maintainer's side of \
         `default/*.loft`:\n  {}",
        bad.join("\n  ")
    );
}

/// loft#1293 — the corpus is a DIFFERENTIAL, so its two halves must run the same SET.
///
/// `tests/wrap.rs` (interpreter) and `tests/native.rs` (native) each pick the entry points of
/// a corpus file that declares no `main`.  They asked different questions: wrap excluded
/// value-returning helpers, native did not, so 165 zero-parameter value-returning functions
/// across 66 files ran on the native pass and on no other — and a differential whose two
/// sides run different code cannot report a divergence between them.
///
/// Both now read `Definition::is_corpus_entry_point`.  This guard is what keeps that true:
/// the drift it replaces was two copies of one rule, and a copy is exactly what a reader
/// adds back when a harness needs "just one more" exclusion.
#[test]
fn both_corpus_halves_pick_entry_points_through_one_predicate() {
    for harness in ["tests/wrap.rs", "tests/native.rs"] {
        let src = std::fs::read_to_string(harness).expect("read the harness");
        assert!(
            src.contains("is_corpus_entry_point()"),
            "{harness} must select corpus entry points through \
             `Definition::is_corpus_entry_point`, so both halves of the differential run the \
             same set (loft#1293)"
        );
        // The rule's own tests, restated locally, are what the drift looked like.
        for restated in ["starts_with(\"n___lambda_\")", "Type::Iterator("] {
            assert!(
                !src.contains(restated),
                "{harness} restates `{restated}` — the entry-point rule has ONE home, and a \
                 second copy is how the two halves came apart (loft#1293)"
            );
        }
    }
}

/// A worked-example citation never reaches the published prose.
///
/// `// Example: @AAA-###` is bookkeeping for `check_doc_drift.sh examples`, addressed to
/// whoever maintains the examples. Rendered into a function's description it is a bare
/// tracker tag with no link and no explanation, which CLAUDE.md § User-facing output
/// rules out — and it leaked onto 27 shipped pages before anyone read one (#1341).
///
/// A citation INSIDE displayed source is correct and stays: the source browser pages show
/// the library's own text, and the tag is part of it. So the guard reads every page with
/// the comment spans removed, which is exactly "the prose a reader is shown".
///
/// This is the end-to-end half of the invariant `documentation::without_example_citations`
/// enforces per line. It reads the COMMITTED HTML, so it also fails when the pages are
/// stale against a generator that has learned to strip them.
#[test]
fn no_worked_example_citation_is_rendered_as_prose() {
    let mut leaked: Vec<String> = Vec::new();
    let mut pages = 0usize;
    for entry in fs::read_dir("doc").expect("doc/ is readable") {
        let path = entry.expect("readable entry").path();
        if path.extension().is_none_or(|e| e != "html") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        pages += 1;
        for prose in strip_comment_spans(&text) {
            for (offset, _) in prose.match_indices("Example: @") {
                let tail: String = prose[offset..].chars().take(60).collect();
                leaked.push(format!("{}: {tail}", path.display()));
            }
        }
    }
    assert!(pages > 100, "expected the generated pages, saw {pages}");
    assert!(
        leaked.is_empty(),
        "a worked-example citation is rendered as prose on {} page section(s); \
         regenerate with `cargo run --bin gendoc`:\n  {}",
        leaked.len(),
        leaked.join("\n  ")
    );
}

/// The page text with `<span class="cm">…</span>` (displayed source comments) removed.
fn strip_comment_spans(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find("<span class=\"cm\">") {
        out.push(rest[..open].to_string());
        rest = &rest[open..];
        match rest.find("</span>") {
            Some(close) => rest = &rest[close + "</span>".len()..],
            None => return out,
        }
    }
    out.push(rest.to_string());
    out
}

/// The control: with the comment spans left in, the source browsers DO carry citations.
///
/// Without this, `no_worked_example_citation_is_rendered_as_prose` would still pass if the
/// stripper ever swallowed the whole page, or if the pages stopped being generated at all.
#[test]
fn the_source_browsers_still_show_the_citations_in_their_source() {
    let text = fs::read_to_string("doc/lib-random-src.html").expect("the random source page");
    assert!(
        text.contains("Example: @RND-001"),
        "the source browser shows the library's own comments verbatim"
    );
    assert!(
        strip_comment_spans(&text)
            .iter()
            .all(|p| !p.contains("Example: @")),
        "…and every one of them sits inside a comment span"
    );
}

/// Pages this repo generates carry no internal tracker tag in their reader-facing prose.
///
/// `@PLN…`, `@P###` and `loft#…` name a plan, a P-issue and an issue. A reader of the
/// published reference cannot open any of them, so a tag in a description is a dead end
/// that reads as the reader's ignorance — CLAUDE.md § User-facing output rules them out
/// of what a command prints, and DOC_QUALITY.md § D says the same for the pages (#1348).
///
/// The exclusions below are the whole difficulty of this guard: each one is a place where
/// the same characters are NOT a leak, and a guard that excluded one case too many would
/// pass over the thing it exists to catch. Each is therefore justified, and
/// [`the_tag_guard_still_sees_a_leak_in_prose`] proves the filter still fires.
#[test]
fn no_internal_tracker_tag_reaches_the_reference_prose() {
    let mut leaked: Vec<String> = Vec::new();
    let mut pages = 0usize;
    for entry in fs::read_dir("doc").expect("doc/ is readable") {
        let path = entry.expect("readable entry").path();
        if path.extension().is_none_or(|e| e != "html") {
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if page_is_exempt(&name) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        pages += 1;
        for prose in strip_comment_spans(&text) {
            for tag in internal_tags(&prose) {
                leaked.push(format!("{}: {tag}", path.display()));
            }
        }
    }
    // The library pages are exempt and there are many of them, so the floor is over what
    // REMAINS — the stdlib sections, the topic chapters and the site pages. A count that
    // collapsed would mean the exemption had swallowed the population.
    assert!(pages > 50, "expected the generated pages, saw {pages}");
    assert!(
        leaked.is_empty(),
        "an internal tracker tag is published as prose in {} place(s). A reader cannot \
         open one — state the rule instead, and keep the tag in the commit or the issue:\n  {}",
        leaked.len(),
        leaked.join("\n  ")
    );
}

/// Pages where these characters are not a leak, and why each is genuinely different.
fn page_is_exempt(name: &str) -> bool {
    match name {
        // The feature catalogue and the print sheet that embeds it: `@F…` IS the subject
        // here, one per catalogue entry. Excluding them costs nothing, because a PLAN tag
        // on those pages is still caught — `internal_tags` never matches `@F`.
        "33-features.html" | "print.html" => false,
        // Single-file app exports. The tags sit in embedded JavaScript and loft source —
        // the page's own runtime shim — which a reader meets as a running program, not as
        // documentation.
        "brick-buster.html"
        | "playground.html"
        | "gallery-run.html"
        | "kernel-differential.html"
        | "kernel-swap.html" => true,
        // Quotes real `--explain` output, which prints `[dead-code lint · @F100]`. The doc
        // is faithful; whether the compiler should print a catalogue id is a question about
        // the OUTPUT, not about this page.
        "34-running.html" => true,
        // Rendered from the registry's stored `api` field, which is derived from each
        // library's own source. Not editable from this repo (#1342 routes the same way).
        n => n.starts_with("lib-"),
    }
}

/// The tag families a READER cannot resolve. `@F`/`@I` are deliberately absent: the
/// feature catalogue is published, so a feature id has somewhere to land.
fn internal_tags(prose: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (i, _) in prose.match_indices('@') {
        let rest = &prose[i + 1..];
        for fam in ["PLN", "PLAN"] {
            if let Some(d) = rest.strip_prefix(fam)
                && d.starts_with(|c: char| c.is_ascii_digit())
            {
                found.push(format!("@{fam}{}", digits(d)));
            }
        }
        // `@P` followed by two or more digits is a P-issue; `@PLN3` is caught above and
        // `@PLAN` cannot reach here because `L` is not a digit.
        if let Some(d) = rest.strip_prefix('P')
            && d.starts_with(|c: char| c.is_ascii_digit())
            && digits(d).len() >= 2
        {
            found.push(format!("@P{}", digits(d)));
        }
    }
    for (i, _) in prose.match_indices("loft#") {
        let d = digits(&prose[i + 5..]);
        if !d.is_empty() {
            found.push(format!("loft#{d}"));
        }
    }
    found
}

fn digits(s: &str) -> String {
    s.chars().take_while(char::is_ascii_digit).collect()
}

/// The control: the filter still finds a tag in ordinary prose.
///
/// Without this, `no_internal_tracker_tag_reaches_the_reference_prose` would pass if
/// `internal_tags` ever stopped matching, or if the exemption list grew to cover
/// everything — the two ways a guard over a shrinking population goes quiet.
#[test]
fn the_tag_guard_still_sees_a_leak_in_prose() {
    let sample = "Number of characters in the text (@PLN110) — the human count.";
    assert_eq!(internal_tags(sample), vec!["@PLN110".to_string()]);
    assert_eq!(
        internal_tags("kept as a shim (loft#1003) and @P259 besides"),
        vec!["@P259".to_string(), "loft#1003".to_string()]
    );
    assert!(
        internal_tags("the dead-code lint is @F100").is_empty(),
        "a feature id is published and resolvable, so it is not in this family"
    );
    assert!(
        !page_is_exempt("stdlib-file-system.html"),
        "the page that carried 42 of them must be covered"
    );
    assert!(
        !page_is_exempt("20-logging.html"),
        "the topic pages must be covered"
    );
}

/// Every `#[ignore]` reason says HOW the test runs instead — by hand (`--ignored`), on a
/// nightly job, or on the platforms that can (`Windows`) — so an ignore is a routing, never
/// a resting place.  Read off the same baseline `A-ignores` reads; a reason that only says
/// WHY (`heavy`, `a measurement`) fails here until it also says where the test runs.
#[test]
fn every_ignore_reason_says_how_it_runs() {
    let baseline = fs::read_to_string("tests/ignored_tests.baseline").expect("read the baseline");
    let markers = [
        "--ignored",
        "miri.yml",
        "ci.yml",
        "on demand",
        "manually",
        "by hand",
        "Windows",
    ];
    let mut silent: Vec<String> = Vec::new();
    for line in baseline.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let (key, reason) = line.split_once('\t').unwrap_or((line, ""));
        if !markers.iter().any(|m| reason.contains(m)) {
            silent.push(format!("{key}\t{reason}"));
        }
    }
    assert!(
        silent.is_empty(),
        "{} ignore reason(s) say why but not how the test runs instead (name `--ignored`, the \
         nightly job, or the platform):\n{}",
        silent.len(),
        silent.join("\n")
    );
}

/// Every subject's PATH pattern claims the sources it names — the map `--changed` reads.
///
/// loft#1520: `changed_filter` looped over `${!SUBJECT_PATHS[@]}`, an array nothing ever
/// assigned, so it ran zero times and `--changed` never selected a subject by path.  The map was
/// not missing — `subject_paths` carried it, complete, and was called by NOBODY.  Both the
/// reference and the map were dead, and nothing noticed because the selection still produced a
/// filterset: `tests/<x>.rs` and the corpus arms kept working, so a run looked targeted while a
/// `src/` edit contributed nothing to it.
///
/// This is NOT the guard that catches that — measured: with the dead loop restored, this test
/// still passes and `changed_selects_subjects_by_path` below is the one that fails.  A test
/// that reads the map directly agrees with a caller that never reads it at all.  What this
/// covers is the other half, the drifted pattern: a row naming a path that no longer exists
/// claims nothing while still reading plausibly.
///
/// It is the sibling of the binary-pattern test further down: that one asks *which binaries
/// does a subject own*, this asks *which subject owns a path*.
#[test]
fn every_subject_claims_the_paths_it_names() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // One representative source path per subject, chosen to be a file that exists — a pattern
    // that stopped matching its own subsystem is the drift this pins.
    let cells: &[(&str, &str)] = &[
        ("src/parser/mod.rs", "parser"),
        ("src/scopes.rs", "scopes"),
        ("src/generation/hoist.rs", "codegen"),
        ("src/state/mod.rs", "runtime"),
        ("src/store.rs", "store"),
        ("src/lsp.rs", "lsp"),
        ("src/ffi_deliver.rs", "wasm"),
        ("src/engine_host.rs", "host"),
        ("doc/claude/TESTING.md", "docs"),
    ];
    for (path, want) in cells {
        assert!(
            root.join(path).exists(),
            "{path} no longer exists — pick another representative for `{want}`, \
             or the cell stops measuring the pattern"
        );
        let script = format!(
            "source scripts/test_subjects.sh; for n in $SUBJECT_NAMES; do \
             p=$(subject_paths \"$n\") || continue; \
             [[ \"{path}\" =~ $p ]] && echo \"$n\"; done"
        );
        let out = std::process::Command::new("bash")
            .args(["-c", &script])
            .current_dir(root)
            .output()
            .expect("bash + scripts/test_subjects.sh");
        let got = String::from_utf8_lossy(&out.stdout);
        let hits: Vec<&str> = got.split_whitespace().collect();
        assert!(
            hits.contains(want),
            "`{path}` must be claimed by subject `{want}`, got {hits:?} — \
             `--changed` selects by this map, so an unclaimed source path means an edit there \
             runs no targeted suite (loft#1520)"
        );
    }

    // And every subject owns a row at all.  `host` had none — it was not a drifted pattern but
    // a missing one, which the per-cell loop above cannot see, since a subject with no row is
    // simply never a candidate answer.  A subject that selects binaries by name but claims no
    // path is unreachable from `--changed` by construction.
    let script = "source scripts/test_subjects.sh; \
                  for n in $SUBJECT_NAMES; do subject_paths \"$n\" >/dev/null || echo \"$n\"; done";
    let out = std::process::Command::new("bash")
        .args(["-c", script])
        .current_dir(root)
        .output()
        .expect("bash + scripts/test_subjects.sh");
    let missing = String::from_utf8_lossy(&out.stdout);
    assert!(
        missing.trim().is_empty(),
        "these subjects have no `subject_paths` row, so no source edit can ever select them: {}",
        missing.trim()
    );
}

/// `--changed` actually selects by path — the guard that would have caught loft#1520.
///
/// The cell test above checks the MAP; this checks the one caller that reads it, and only the
/// second one could have failed on the broken build.  `changed_filter` looped over
/// `${!SUBJECT_PATHS[@]}`, an array nothing assigned, so the map was correct and unreachable at
/// the same time — a guard that reads `subject_paths` directly agrees with a dead caller.
///
/// `changed_paths` is redefined after sourcing, so the diff under test is a literal list rather
/// than whatever the working tree happens to hold.  The second cell is the fail-safe control:
/// an unclaimed `src/` path beside a test file must widen the run, because the false green the
/// issue named is precisely that the test file narrows it alone.
#[test]
fn changed_selects_subjects_by_path() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let run = |diff: &[&str]| -> (bool, String, String) {
        let list = diff.join(" ");
        let script = format!(
            "source scripts/test_subjects.sh; changed_paths() {{ printf '%s\\n' {list}; }}; \
             changed_filter HEAD"
        );
        let out = std::process::Command::new("bash")
            .args(["-c", &script])
            .current_dir(root)
            .output()
            .expect("bash + scripts/test_subjects.sh");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    let (ok, stdout, stderr) = run(&["src/parser/mod.rs"]);
    assert!(
        ok && stderr.contains("subjects parser"),
        "a parser edit must select the `parser` subject; got rc={ok} stderr={stderr:?} \
         stdout={stdout:?} (loft#1520)"
    );
    assert!(
        stdout.contains("binary(parse_errors)"),
        "the selected subject must reach its binaries, not just name itself: {stdout:?}"
    );

    let (ok, stdout, stderr) = run(&["src/json.rs", "tests/wrap.rs"]);
    assert!(
        !ok && stderr.contains("no subject claims src/json.rs"),
        "an unclaimed `src/` path must widen the run — otherwise the test file beside it \
         narrows the selection to itself and reports a targeted pass; got rc={ok} \
         stderr={stderr:?} stdout={stdout:?}"
    );
}

/// @PLN159 phase H — every test binary belongs to a subject.
///
/// `scripts/find_problems.sh --subject <name>` and `--changed` select binaries through
/// the `SUBJECT_PATTERNS` map in `scripts/test_subjects.sh`; a binary no pattern matches
/// is invisible to both — it still runs in the curated and full sets, but never in the
/// iteration loop that is supposed to catch a regression in seconds.  The map's own
/// `unmatched_binaries` report existed (printed by `--list-subjects`) and was checked by
/// nobody, so the guard asks it here: a new `tests/<name>.rs` either matches a subject
/// or extends the map in the same commit.  Goes red for exactly one reason — a name in
/// that report — which the message prints verbatim.
#[test]
fn every_test_binary_matches_a_subject() {
    // Two different failures wear one exit code here, and only one of them is this guard's
    // subject.  The script REFUSING (a binary no subject reaches) prints the name on stdout;
    // the shell failing to RUN prints nothing at all.  The Windows daily hit the second and
    // reported it as the first, twice — nextest's own retry saw the same thing — and a
    // windows-probe run of the identical command passed with exit 0 and zero unmatched
    // binaries (bash 5.3.15, Cygwin), so the cause is load on the runner rather than
    // anything in the map.  So: a non-zero exit with BOTH streams empty is retried once,
    // and either way the message names what actually happened.  The retry cannot mask a
    // genuine finding, because a genuine finding has stdout.
    // ⚠ On Windows a bare `bash` is NOT Git Bash — it resolves to
    // `C:\Windows\System32\bash.exe`, the WSL launcher, which with no distribution
    // installed prints "Windows Subsystem for Linux has no installed distributions" to
    // STDOUT (UTF-16) and exits 1.  That is the whole story behind this test failing twice
    // on the Windows daily while a windows-probe run of the identical command passed: the
    // probe ran INSIDE Git Bash, where `bash` resolves to Git Bash first, and the test
    // process inherits a different PATH order.  The 20-odd other suites here spawn `sh`,
    // which System32 does not provide, so this was the only site exposed.  `sh` is not the
    // cure though — `test_subjects.sh` needs `BASH_SOURCE` and `[[ ]]`, and `/bin/sh` on
    // Linux is dash.
    let bash = || -> std::ffi::OsString {
        #[cfg(windows)]
        {
            let mut roots: Vec<std::path::PathBuf> = Vec::new();
            for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
                if let Some(v) = std::env::var_os(var) {
                    roots.push(std::path::PathBuf::from(v));
                }
            }
            roots.push(std::path::PathBuf::from(r"C:\Program Files"));
            for r in roots {
                let c = r.join("Git").join("bin").join("bash.exe");
                if c.is_file() {
                    return c.into_os_string();
                }
            }
        }
        std::ffi::OsString::from("bash")
    };
    let run = || {
        std::process::Command::new(bash())
            .args([
                "-c",
                "source scripts/test_subjects.sh && unmatched_binaries",
            ])
            .output()
            .expect("bash + scripts/test_subjects.sh")
    };
    let mut out = run();
    let mut retried = false;
    if !out.status.success() && out.stdout.is_empty() && out.stderr.is_empty() {
        std::thread::sleep(std::time::Duration::from_secs(2));
        out = run();
        retried = true;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let unmatched: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    // Everything the next failure needs, because the first version reported only stderr —
    // and the Windows daily failed it twice with stderr EMPTY, which made the message say
    // literally "scripts/test_subjects.sh failed: " and left the cause undiagnosable.  A
    // windows-probe run of the identical command passed (exit 0, no unmatched binaries),
    // so the cause is load-dependent rather than a property of the script; naming the exit
    // code and what the run did produce is what turns the next occurrence into evidence.
    assert!(
        out.status.success(),
        "scripts/test_subjects.sh failed: exit {:?}\n  stderr ({} bytes): {}\n  stdout ({} bytes): {}\n  \
         (an empty stderr with a non-zero exit is the shell dying rather than the script \
         refusing — on Windows this ran green in isolation under windows-probe; \
         retried_once={retried})",
        out.status.code(),
        out.stderr.len(),
        String::from_utf8_lossy(&out.stderr),
        out.stdout.len(),
        stdout.lines().take(5).collect::<Vec<_>>().join(" | ")
    );
    assert!(
        unmatched.is_empty(),
        "{} test binar{} matched by no subject in scripts/test_subjects.sh — add a pattern to \
         `subject_patterns` (or join an existing binary; TESTING.md § Subjects): {}",
        unmatched.len(),
        if unmatched.len() == 1 {
            "y is"
        } else {
            "ies are"
        },
        unmatched.join(", ")
    );
}

/// The nightly workflow's gate CLASS lives on the job, and every list that reads it agrees.
///
/// `.github/workflows/miri.yml` runs fourteen checks and then makes two decisions about
/// them: which reds open the tracked `nightly-failure` issue (`notify`), and which appear
/// in the daily digest (`daily-status`).  Both were written as hand-maintained job lists,
/// and the digest closed with a third copy of the first list in prose — one question with
/// three decoders and nothing keeping them equal.
///
/// Two had already drifted.  `valgrind` was in NONE of them, so memcheck over the shipped
/// release binary reported to nobody: on 2026-09-07 it went red beside `poison` and
/// `debug-asserts`, the filed issue named only the other two, and because `notify` closes
/// the issue when every gate it CAN SEE is green, the next green run would have closed it
/// with memcheck still failing.  That is the failure mode the job's own comment warns
/// about — a gate stops gating without ever going red — reached by the route the comment
/// did not cover: not a gate added and forgotten, a gate added to no list at all.
///
/// So the class is declared once, on the job, as `# @nightly-class: unsound|report`, and
/// this test is what makes the three readers derive from it rather than repeat it.  It
/// also fails on a job carrying NO marker, which is the half that matters for the next
/// gate somebody adds: the question has to be answered for the file to compile.
///
/// `unsound` means a red says the language is broken — hard UB, a data race, a use-after-
/// free, an out-of-bounds or uninitialised read, a violated internal invariant.  `report`
/// means everything else: toolchain drift, doc index, library health, the leak baseline,
/// coverage replayed for release evidence.  Reclassifying is a one-word edit here.
#[test]
fn nightly_gate_classes_drive_every_list_that_reads_them() {
    let wf = std::fs::read_to_string(".github/workflows/miri.yml").expect("read miri.yml");
    let lines: Vec<&str> = wf.lines().collect();
    let jobs_at = lines
        .iter()
        .position(|l| *l == "jobs:")
        .expect("miri.yml has a `jobs:` key");

    // Job keys are the only two-space `name:` lines below `jobs:`; `on:`'s own `schedule:`
    // sits above it and is why this starts at `jobs_at` rather than scanning the file.
    let is_job_key = |l: &str| {
        l.len() > 3
            && l.starts_with("  ")
            && !l.starts_with("   ")
            && l.ends_with(':')
            && l[2..l.len() - 1]
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    let job_starts: Vec<usize> = (jobs_at + 1..lines.len())
        .filter(|i| is_job_key(lines[*i]))
        .collect();
    assert!(
        job_starts.len() > 10,
        "found only {} jobs in miri.yml — the key parser stopped matching",
        job_starts.len()
    );

    let job_name = |i: usize| lines[i][2..lines[i].len() - 1].to_string();
    let block = |n: usize| -> &[&str] {
        let start = job_starts[n];
        let end = job_starts.get(n + 1).copied().unwrap_or(lines.len());
        &lines[start..end]
    };

    // The two CONSUMERS carry no class of their own: they are the readers, not gates.
    let consumers = ["notify", "daily-status"];
    let mut unsound: Vec<String> = Vec::new();
    let mut classified: Vec<String> = Vec::new();
    let mut unmarked: Vec<String> = Vec::new();
    for (n, &i) in job_starts.iter().enumerate() {
        let name = job_name(i);
        let marker = block(n)
            .iter()
            .find_map(|l| l.trim().strip_prefix("# @nightly-class:"))
            .map(|rest| {
                rest.split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_string()
            });
        match (consumers.contains(&name.as_str()), marker.as_deref()) {
            (true, None) => {}
            (true, Some(c)) => panic!(
                "job `{name}` reads the classes and must not carry one itself, but it says \
                 `@nightly-class: {c}`"
            ),
            (false, None) => unmarked.push(name),
            (false, Some(c)) => {
                assert!(
                    c == "unsound" || c == "report",
                    "job `{name}` has `@nightly-class: {c}` — the only classes are \
                     `unsound` (a red says the language is broken) and `report`"
                );
                if c == "unsound" {
                    unsound.push(name.clone());
                }
                classified.push(name);
            }
        }
    }
    assert!(
        unmarked.is_empty(),
        "these nightly jobs carry no `# @nightly-class:` marker, so nothing decides whether \
         a red opens the tracked issue or only reaches the digest — add one as the first \
         line of the job block:\n  {}",
        unmarked.join("\n  ")
    );

    // `needs:` may wrap across lines (the digest's list does), so read the block from the
    // `needs:` line to the closing bracket rather than the line alone.
    let needs_of = |n: usize| -> Vec<String> {
        let b = block(n);
        let at = b
            .iter()
            .position(|l| l.trim_start().starts_with("needs:"))
            .unwrap_or_else(|| panic!("job `{}` has no `needs:`", job_name(job_starts[n])));
        let mut joined = String::new();
        for l in &b[at..] {
            joined.push_str(l);
            if l.contains(']') {
                break;
            }
        }
        joined[joined.find('[').expect("a needs list") + 1..joined.rfind(']').expect("a `]`")]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let index_of = |name: &str| {
        job_starts
            .iter()
            .position(|&i| job_name(i) == name)
            .unwrap_or_else(|| panic!("miri.yml has no `{name}` job"))
    };

    let mut want_notify = unsound.clone();
    want_notify.sort();
    let mut got_notify = needs_of(index_of("notify"));
    got_notify.sort();
    assert_eq!(
        got_notify, want_notify,
        "`notify.needs` disagrees with the `@nightly-class: unsound` markers. A gate it \
         cannot see does not merely fail to file — its silence reads as green and \
         AUTO-CLOSES the tracked issue another gate opened."
    );

    let mut want_digest = classified.clone();
    want_digest.sort();
    let mut got_digest = needs_of(index_of("daily-status"));
    got_digest.sort();
    assert_eq!(
        got_digest, want_digest,
        "`daily-status.needs` omits a classified job, so the digest that calls itself \
         `the one place to look` is not one."
    );

    // The digest's closing sentence names the same set in prose. It is the copy that went
    // stale silently before — it was still naming four gates when `notify` watched five.
    let digest = block(index_of("daily-status"));
    let foot_from = digest
        .iter()
        .position(|l| l.contains("Gates that FILE an issue"));
    let foot_to = digest
        .iter()
        .position(|l| l.contains("Everything else reports"));
    let footer: String = match (foot_from, foot_to) {
        (Some(a), Some(b)) if b >= a => digest[a..=b].join(" "),
        _ => String::new(),
    };
    assert!(
        !footer.is_empty(),
        "the daily digest no longer says which gates file an issue"
    );
    for g in &unsound {
        assert!(
            footer.contains(g.as_str()),
            "the digest's closing sentence does not name `{g}`, which IS in the \
             `unsound` class: {footer}"
        );
    }
    for g in &classified {
        if !unsound.contains(g) {
            assert!(
                !footer.contains(g.as_str()),
                "the digest's closing sentence names `{g}` as a gate that files an issue, \
                 but its marker says `report`: {footer}"
            );
        }
    }
}
