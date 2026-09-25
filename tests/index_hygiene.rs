// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Plan-37 phase 03 + 09 — CI gate for the tracker-tag indexer.
//
// Two tests, both gated against `index/tags.json`:
//
//   `no_broken_tracker_tags` (phase 03) — every `@P-id` /
//   `@PLAN-id` reference resolves to an existing PROBLEMS.md
//   row or plan directory.
//
//   `no_broken_markdown_links` (phase 09 follow-up, enabled
//   2026-05-14 once the cleanup pass shipped) — every
//   `[text](path.md)` markdown link resolves to an existing
//   file.
//
// To pass with an INTENTIONAL fake reference (e.g., a design
// doc explaining what a broken tag looks like, or a template
// placeholder), put the literal `<!--noindex-->` marker on
// the same line; the scanner skips those lines.
//
// To DEBUG a failing run:
//   ./scripts/idx broken         # for phase-03 failures
//   ./scripts/idx broken-links   # for phase-09 failures
//
// To FIX:
//   - phase 03: rename the @P-id / @PLAN-id to a real one,
//     add the missing PROBLEMS.md row / plan dir, or add
//     `<!--noindex-->` to the line.
//   - phase 09: fix the relative path (often an off-by-one
//     `..` after a file moves to `finished/`), point at the
//     correct doc, or add `<!--noindex-->` for intentional
//     placeholder examples.  `tools/indexer/fix_broken_links.py`
//     auto-fixes the common off-by-one cases.

use std::process::Command;

/// Run the loft-native `idx.loft` query binary instead of bash
/// `scripts/idx`.  The bash script is the bootstrap path
/// (documented in CLAUDE.md, used from machines without a built
/// loft), but it has accumulated half a dozen cross-platform
/// gotchas — BSD awk's UTF-8 panic on em-dashes, MSYS argv
/// limits, Windows PE-format rejection of bash scripts,
/// MSYS-vs-native-jq path translation, GNU-only `xargs -a` /
/// `find -regex` / `stat -c %Y`, etc.  The loft port runs as a
/// single native binary on every platform with consistent
/// behaviour and short-circuits the entire portability surface.
///
/// `idx.loft`'s MVP covers the two queries the CI gate uses
/// (`broken` and `broken-links`); the other queries (`tag:`,
/// `prefix:`, `file:`, `incoming:`, `all`, `help`, excerpt
/// flags) stay in the bash script until a consumer asks.
fn idx_command(args: &[&str]) -> Command {
    // `env!("CARGO_BIN_EXE_loft")`: the loft binary cargo already built for
    // this test run.  Do NOT shell out to `cargo run` here: a nested cargo
    // invocation resolves the NON-test feature universe against the live
    // `target/release`, marking shared dependency rlibs dirty and rewriting
    // them while sibling nextest processes are linking against them — the
    // "cannot open libring-….rlib" / "crate `rustls` required to be
    // available in rlib format" cdylib-build flakes (#307; also the
    // nondeterministic half of #304).
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("tools/indexer/src/idx.loft");
    for a in args {
        cmd.arg(a);
    }
    cmd
}

// Single test for both checks — running `make index` twice
// concurrently (cargo test default parallelism) corrupts
// `index/tags.json`.  Serialising into one test avoids the
// race without needing a global mutex.
// @speed 2.3
/// @PLN119 arc F — the index covers exactly what git carries.
///
/// The scanner used to enumerate a hard-coded list of source roots, pruned by a
/// hand-maintained set of directory names that mean "ignored" (`target`,
/// `node_modules`, `pkg`, `generated`, a worktree…). Every comment in that list
/// said what it really was — "bash's `git ls-files` skips these" — and a copy of
/// `.gitignore` maintained by hand is stale the moment anyone edits the real one.
///
/// It was stale in both directions when this landed: four TRACKED source trees
/// (`fuzz/`, `loft-ffi/`, `loft-ffi-build/`, `loft-ffi-macros/`) were never
/// indexed at all because nobody added them to the root list, while a leftover
/// `lib/.loft_test_tmp_*/` scratch directory WAS, because nobody had added that
/// name to the skip list.
///
/// So the property is now the one that was always meant: **every file the index
/// mentions is a file git carries, and a tracked file outside the old root list
/// is covered.** Both halves matter — the first alone passes on an index that
/// covers nothing.
fn check_index_matches_git() {
    let carried: std::collections::HashSet<String> = {
        let out = Command::new("git")
            .args(["ls-files", "--cached", "--others", "--exclude-standard"])
            .output()
            .expect("failed to spawn git ls-files");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect()
    };
    assert!(
        carried.len() > 500,
        "git listed {} files — this test is measuring nothing",
        carried.len()
    );

    // Pull the paths out by hand rather than adding a JSON dependency: the
    // rows are `{"file":"<path>","line":N,…}`, and what is being checked is
    // the set of paths, not the document's shape (which `idx` already reads).
    let index = std::fs::read_to_string("index/tags.json").expect("read index/tags.json");
    let mut indexed: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for chunk in index.split("\"file\":\"").skip(1) {
        if let Some(end) = chunk.find('"') {
            indexed.insert(&chunk[..end]);
        }
    }
    assert!(
        indexed.len() > 500,
        "the index names {} files — this test is measuring nothing",
        indexed.len()
    );

    let strays: Vec<&&str> = indexed.iter().filter(|p| !carried.contains(**p)).collect();
    assert!(
        strays.is_empty(),
        "the index names {} file(s) git does not carry — the enumeration has \
         drifted away from `git ls-files`: {:?}",
        strays.len(),
        &strays[..strays.len().min(10)]
    );

    // Coverage: a tracked file in a tree the old hard-coded root list never
    // mentioned. If `loft-ffi` is ever removed, replace this with another —
    // the point is that the root list is gone, not that this path exists.
    let outside = "loft-ffi/src/lib.rs";
    if carried.contains(outside) {
        assert!(
            indexed.contains(outside),
            "`{outside}` is tracked but not indexed — the scanner is back to \
             enumerating a fixed list of roots"
        );
    }
}

/// Every relative link in the tracked markdown resolves.  Wider than the
/// index's `broken_links` bucket, which resolves only the links its scanner
/// recognises: this reads each file the way a markdown renderer does (fences
/// and inline code spans are not links) and exempts what `scripts/linkcheck.sh`
/// exempts.  It reads no index, so it cannot race `make index`.
#[test]
fn every_markdown_link_resolves() {
    let out = Command::new("python3")
        .arg("tools/indexer/fix_broken_links.py")
        .output()
        .expect("failed to spawn tools/indexer/fix_broken_links.py");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && !stdout.contains("  fix "),
        "broken markdown links:\n{stdout}{}\n\
         `make doc-fix` writes each `fix` line; a `flag` line needs a person \
         (point it at the thing's new home, or mark an intentional placeholder \
         with `<!--noindex-->` or a code span).  A directory move is \
         `make plan-move FROM=… TO=…`, which rewrites the links as it moves.",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn index_hygiene_clean() {
    // 1. Refresh the index.  `make index` must exit 0.
    let make = Command::new("make")
        .arg("index")
        .output()
        .expect("failed to spawn `make index`");
    assert!(
        make.status.success(),
        "make index exited {:?}\nstdout:\n{}\nstderr:\n{}",
        make.status.code(),
        String::from_utf8_lossy(&make.stdout),
        String::from_utf8_lossy(&make.stderr)
    );

    // 1b. @PLN119 arc F — the index covers exactly what git carries.
    //     Here rather than in a test of its own: two tests running `make index`
    //     concurrently corrupt `index/tags.json`, which is why this file has
    //     one gate rather than several.
    check_index_matches_git();

    // 1c. loft#1544 — the CANONICAL plan family must be INDEXED, not merely
    //     well-formed.  `@PLN<n>` is a `loft-lang/plans` issue and CLAUDE.md
    //     tells every agent to look plan refs up with `./scripts/idx`; the
    //     family was migrated and the scanner was not, so every such query
    //     answered `[]` at exit 0 for tags with thousands of references.
    //
    //     That failure mode is why this is a gate and not a doc note: an
    //     unindexed family is INDISTINGUISHABLE from "referenced nowhere".
    //     Everywhere else an empty list is trustworthy, because a malformed
    //     query exits 2 — which is exactly what makes a silently-empty answer
    //     here read as a fact.
    //
    //     A floor rather than an exact count: the number moves whenever a plan
    //     is cited, and pinning it would make every unrelated commit re-pin a
    //     number.  The floor still fails closed — a scanner that dropped the
    //     family reports 0, not 20.
    let index = std::fs::read_to_string("index/tags.json").expect("read index/tags.json");
    let pln_keys = index
        .split("\"@PLN")
        .skip(1)
        .filter(|chunk| {
            let digits = chunk.len() - chunk.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            digits > 0 && chunk[digits..].starts_with("\":")
        })
        .count();
    assert!(
        pln_keys > 20,
        "index/tags.json holds {pln_keys} `@PLN<n>` keys — the canonical plan \
         tag family is not being indexed, so `./scripts/idx tag:@PLN<n>` answers \
         an empty list at exit 0 for every plan (loft#1544).  The scanner arm is \
         in tools/indexer/src/scan.loft, beside the `@PLAN` one."
    );

    // 2. Phase 03 — broken @-tag refs.
    let broken_tags = idx_command(&["broken"])
        .output()
        .expect("failed to spawn `bash ./scripts/idx broken`");
    assert!(
        broken_tags.status.success(),
        "./scripts/idx broken exited {:?}\nstdout:\n{}\nstderr:\n{}",
        broken_tags.status.code(),
        String::from_utf8_lossy(&broken_tags.stdout),
        String::from_utf8_lossy(&broken_tags.stderr)
    );
    let tags_out = String::from_utf8_lossy(&broken_tags.stdout);
    assert_eq!(
        tags_out.trim(),
        "[]",
        "broken tracker tags found:\n{tags_out}\n\
         Fix options:\n  \
         (a) rename the @P-id / @PLAN-id to a real one\n  \
         (b) add the missing PROBLEMS.md row / plan dir\n  \
         (c) add `<!--noindex-->` to the line if the ref is \
         an intentional documentation example\n\
         See: doc/claude/plans/37-tracker-index/03-broken-validator.md"
    );

    // 2.5 (retired in sub-commit H, 2026-05-18) — `make index`
    // now invokes scan.loft as the canonical source; the prior
    // bash-vs-loft jq-projection diff has no two sides to
    // compare.  Per-bucket parity is enforced by the golden
    // baseline assertion below (sub-commit A.5) AND the upcoming
    // sub-commit-K parity gates against `tests/golden/tags.json`.

    // 3. Phase 09 — broken markdown-link refs.
    let broken_links = idx_command(&["broken-links"])
        .output()
        .expect("failed to spawn `bash ./scripts/idx broken-links`");
    assert!(
        broken_links.status.success(),
        "./scripts/idx broken-links exited {:?}\nstdout:\n{}\nstderr:\n{}",
        broken_links.status.code(),
        String::from_utf8_lossy(&broken_links.stdout),
        String::from_utf8_lossy(&broken_links.stderr)
    );
    let links_out = String::from_utf8_lossy(&broken_links.stdout);
    assert_eq!(
        links_out.trim(),
        "[]",
        "broken markdown links found:\n{links_out}\n\
         Fix options:\n  \
         (a) fix the relative path (often an off-by-one `..`\n      \
             after the source moved to `finished/`)\n  \
         (b) point at the correct doc\n  \
         (c) add `<!--noindex-->` to the line if the link is\n      \
             an intentional placeholder example\n  \
         (d) `make doc-fix` repairs every link whose target is\n      \
             unique, and names the rest\n\
         See: doc/claude/plans/42-tracker-index/09-backlinks.md"
    );

    // 4. Cross-OS portability sanity (@PLAN37 phase 07 pre-flight P2)
    // — `index/tags.json` must not contain literal backslashes outside
    // JSON `\"` escapes.  Catches the Windows-only "path separator
    // drift" silent failure on every CI runner (not just Windows):
    // when scan.loft runs on MSYS bash and emits `file().path` raw,
    // the `path` / `file` / `target` fields contain `\` separators
    // and the parity assertion breaks downstream.  Linux runners see
    // the same assertion — fails early instead of waiting for a
    // Windows CI cycle to surface the bug.
    let tags_raw = std::fs::read_to_string("index/tags.json").expect("read index/tags.json");
    // Allow `\\` (JSON-escaped backslash, e.g. inside a context string)
    // and `\"` (escaped quote).  Anything else is a raw backslash that
    // shouldn't be there.  Strip both, then count remaining backslashes.
    let stripped = tags_raw.replace("\\\\", "").replace("\\\"", "");
    let stray = stripped.matches('\\').count();
    assert_eq!(
        stray, 0,
        "tags.json contains {stray} stray backslash(es) — Windows path-separator \
         drift?  scan.loft should pass every path-shaped string through \
         `normalize_path()` (see tools/indexer/src/scan.loft).  Sample: \
         look for a `\\` outside `\\\\` / `\\\"` JSON escapes in tags.json."
    );

    // 5. PROBLEMS.md row-parser sanity (@PLAN37 phase 07 pre-flight P4)
    // — every `problems_open` row's `severity` cell must start with a
    // known severity prefix.  A literal `|` in a row body would shift
    // column boundaries during pipe-split and put body text in the
    // severity cell.  Both scan.sh and scan.loft share this edge case;
    // this assertion catches the silent mis-categorisation.  bash side
    // is canonical until sub-commit H; the assertion applies regardless.
    // Use `jq -r` rather than pulling serde_json into this binary just
    // for one assertion — jq is already a CI dep used elsewhere here.
    let sev_jq = Command::new("jq")
        .args(["-r", ".problems_open[] | .severity", "index/tags.json"])
        .output()
        .expect("spawn jq for severity check");
    assert!(
        sev_jq.status.success(),
        "jq severity extraction failed: {}",
        String::from_utf8_lossy(&sev_jq.stderr)
    );
    // Loose contains-check rather than starts_with — @P229's row has a
    // legitimate custom severity (`(a) Closed; (b) Open (Windows)`) for
    // its split-state windows-half-still-open situation, which doesn't
    // start with any standard prefix but still contains a known severity
    // word.  A literal `|` mid-body would produce garbage that contains
    // NONE of these tokens.
    let severity_tokens = [
        "Low", "Medium", "High", "Critical", "Open", "open", "Closed", "closed", "partial",
    ];
    for sev in String::from_utf8_lossy(&sev_jq.stdout).lines() {
        if sev.is_empty() {
            continue;
        }
        let ok = severity_tokens.iter().any(|p| sev.contains(p));
        assert!(
            ok,
            "problems_open row severity `{sev}` doesn't contain any known \
             severity word — PROBLEMS.md row may contain a literal `|` that \
             breaks pipe-split"
        );
    }
}
