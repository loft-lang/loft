// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN119 arc F — `lib/git`, the plan's first real consumer.
//
// The library answers questions about a repository, so the only honest oracle is
// GIT ITSELF: every assertion here runs the real command beside the library call
// and requires the two to agree. A test that only checked "the answer looks like
// a sha" would pass on a library that read the wrong repository, which is exactly
// the bug arc F actually found.
//
// The repository is built by the test rather than borrowed from the checkout —
// its history has to contain the awkward shapes on purpose (a subject with a TAB
// in it, a rename, a binary file), and a real checkout's history is whatever it
// happens to be that day.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = base.join("loft-lib-git").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run git in `dir` and answer its stdout, trimmed of the trailing newline.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("run git");
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

/// Build a repository whose history contains the shapes that break a naive
/// reader, and answer its path.
fn repo(name: &str) -> Option<PathBuf> {
    if Command::new("git").arg("--version").output().is_err() {
        return None;
    }
    let dir = scratch(name);
    let run = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
            .output()
            .expect("run git");
        assert!(
            ok.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&ok.stderr)
        );
    };
    Command::new("git")
        .arg("init")
        .arg("-q")
        .arg("-b")
        .arg("main")
        .arg(&dir)
        .output()
        .expect("git init");
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").expect("write a");
    run(&["add", "a.txt"]);
    // A subject with a TAB in it. `tools/viewer/refresh.sh` splits `git log` on
    // TAB, so this line is what silently shifts its fields — and the reason
    // `lib/git` asks git for `%x1f` separators instead.
    run(&["commit", "-q", "-m", "first\tcommit with a tab"]);
    std::fs::write(dir.join("b.txt"), "héllo ✓\n").expect("write b");
    run(&["add", "b.txt"]);
    run(&[
        "commit",
        "-q",
        "-m",
        "second — ünïcøde and \"quotes\" and 100%",
    ]);
    Some(dir)
}

/// Run `program` against `lib/` in `dir`, with `placement` forced.
fn run_loft(dir: &Path, program: &str, placement: &str) -> (String, String) {
    let lib = dir.join("libs");
    let src = lib.join("git").join("src");
    std::fs::create_dir_all(&src).expect("create lib dir");
    // A copy of the real library with only its manifest edited, so the test
    // exercises the shipped source rather than a paraphrase of it.
    let real = workspace_root().join("lib").join("git");
    std::fs::copy(real.join("src").join("git.loft"), src.join("git.loft")).expect("copy source");
    let manifest = std::fs::read_to_string(real.join("loft.toml")).expect("read manifest");
    let manifest = manifest
        .lines()
        .map(|l| {
            if l.starts_with("placement") {
                format!("placement = \"{placement}\"")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(lib.join("git").join("loft.toml"), manifest).expect("write manifest");

    // `#cwd` so the program runs in the repository rather than beside its own
    // source — which is what every tool in this tree wants, and what the viewer
    // driver does.
    let path = dir.join("probe.loft");
    std::fs::write(&path, format!("#cwd\npub use git::*;\n{program}")).expect("write probe");
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg("--lib")
        .arg(&lib)
        .arg(&path)
        .current_dir(dir)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_NATIVE_LIBS", "1")
        .output()
        .expect("run loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Every query, against git's own answer — and under BOTH placements, because
/// this library is the plan's dogfood: its answers are `vector<Commit>` and
/// `vector<Change>`, which is exactly the shape arc B's arena carries.
#[test]
fn every_query_answers_what_git_answers() {
    let Some(dir) = repo("queries") else {
        eprintln!("skip: no git on this machine");
        return;
    };
    let program = "fn main() {\n\
                   \x20   println(\"branch={branch()}\");\n\
                   \x20   h = head();\n\
                   \x20   println(\"sha={h.sha}\");\n\
                   \x20   println(\"subject={h.subject}\");\n\
                   \x20   println(\"hasmain={has_ref(\"main\")}\");\n\
                   \x20   println(\"hasnope={has_ref(\"no-such-ref\")}\");\n\
                   \x20   lg = log(10);\n\
                   \x20   println(\"logn={len(lg)}\");\n\
                   \x20   for c in lg { println(\"log={c.sha}|{c.date}|{c.subject}\"); }\n\
                   \x20   for s in log_shas(10) { println(\"shas={s}\"); }\n\
                   }\n";
    for placement in ["inproc", "process"] {
        let (out, err) = run_loft(&dir, program, placement);
        let field = |k: &str| -> String {
            out.lines()
                .find_map(|l| l.strip_prefix(k))
                .unwrap_or_else(|| panic!("no {k} line under {placement}: {out:?} / {err}"))
                .to_string()
        };
        assert_eq!(
            field("branch="),
            git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        );
        assert_eq!(field("sha="), git(&dir, &["rev-parse", "--short", "HEAD"]));
        // The subject has a `%`, an em-dash, quotes and multi-byte characters,
        // so this compares the awkward one rather than a well-behaved one.
        assert_eq!(field("subject="), git(&dir, &["log", "-1", "--pretty=%s"]));
        assert_eq!(field("hasmain="), "true");
        assert_eq!(field("hasnope="), "false");
        assert_eq!(field("logn="), "2");

        // The full log, field by field, against git's own rendering. The FIRST
        // commit's subject contains a TAB — a tab-separated reader loses
        // everything after it, which is the bug this library's `%x1f` avoids.
        let want: Vec<String> = git(&dir, &["log", "-10", "--pretty=%h|%ad|%s", "--date=short"])
            .lines()
            .map(str::to_string)
            .collect();
        let got: Vec<String> = out
            .lines()
            .filter_map(|l| l.strip_prefix("log="))
            .map(str::to_string)
            .collect();
        assert_eq!(got, want, "under {placement}");
        assert!(
            got[1].contains("first\tcommit with a tab"),
            "the awkward subject is not in the history, so this proves nothing: {got:?}"
        );

        let shas: Vec<String> = out
            .lines()
            .filter_map(|l| l.strip_prefix("shas="))
            .map(str::to_string)
            .collect();
        assert_eq!(
            shas,
            git(&dir, &["log", "-10", "--pretty=%h"])
                .lines()
                .collect::<Vec<_>>()
        );
    }
}

/// The working-tree and diff queries, which is where a status code or a rename
/// is mis-read rather than lost.
#[test]
fn the_working_tree_and_diff_queries_agree_with_git() {
    let Some(dir) = repo("worktree") else {
        eprintln!("skip: no git on this machine");
        return;
    };
    // A modification, an untracked file, and a rename — the three shapes
    // `refresh.sh` handles by hand, one of which (the rename) it got wrong once.
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\nfour\n").expect("modify a");
    std::fs::write(dir.join("new.txt"), "fresh\n").expect("add new");
    let program = "fn main() {\n\
                   \x20   for c in uncommitted() { println(\"unc={c.status}|{c.path}\"); }\n\
                   \x20   for c in changed(\"main\") { println(\"chg={c.status}|{c.path}\"); }\n\
                   \x20   for n in changed_names(\"main\") { println(\"nam={n}\"); }\n\
                   \x20   ab = ahead_behind(\"main\");\n\
                   \x20   println(\"ab={ab.ahead}|{ab.behind}\");\n\
                   \x20   sh = log_shas(1);\n\
                   \x20   for s in numstat(sh[0]) { println(\"num={s.adds}|{s.dels}|{s.path}\"); }\n\
                   \x20   println(\"showlen={size(show(sh[0])) > 40}\");\n\
                   }\n";
    for placement in ["inproc", "process"] {
        let (out, err) = run_loft(&dir, program, placement);
        let lines = |k: &str| -> Vec<String> {
            out.lines()
                .filter_map(|l| l.strip_prefix(k))
                .map(str::to_string)
                .collect()
        };
        // `git status --short` prints "<XY> <path>"; the library strips the
        // spaces out of the code and keeps the rest as the path.
        let want: Vec<String> = git(&dir, &["status", "--short"])
            .lines()
            .map(|l| format!("{}|{}", l[0..2].replace(' ', ""), &l[3..]))
            .collect();
        assert_eq!(lines("unc="), want, "uncommitted, under {placement}: {err}");
        assert!(
            want.iter().any(|l| l.starts_with("??")),
            "the probe repository has no untracked file, so this proves nothing"
        );
        // HEAD *is* main here, so the two diff queries are empty and
        // ahead/behind is zero — the fresh-repository shape `refresh.sh` needed
        // its own branch for.
        assert_eq!(lines("chg="), Vec::<String>::new());
        assert_eq!(lines("nam="), Vec::<String>::new());
        assert_eq!(lines("ab="), vec!["0|0".to_string()]);

        let head = git(&dir, &["log", "-1", "--pretty=%h"]);
        let want: Vec<String> = git(&dir, &["show", &head, "--numstat", "--pretty=format:"])
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.split('\t').collect::<Vec<_>>().join("|"))
            .collect();
        assert_eq!(lines("num="), want, "numstat, under {placement}");
        assert_eq!(lines("showlen="), vec!["true".to_string()]);
    }
}

/// @PLN119 arc F — the viewer's state dump, which is what `lib/git` was built
/// for.
///
/// `tools/viewer/refresh.loft` replaced 135 lines of bash that existed only
/// because loft could not call `git` — and with it the dashboard's dependency on
/// `jq`. The port was proven against the script it replaced (four JSON documents
/// semantically identical, 57 diffs byte-identical) at the moment of the swap;
/// the script is gone now, so the durable oracle is git itself, the same one the
/// queries above use.
#[test]
fn the_viewer_state_dump_reports_what_git_reports() {
    let Some(dir) = repo("viewer") else {
        eprintln!("skip: no git on this machine");
        return;
    };
    std::fs::write(dir.join("c.txt"), "untracked\n").expect("write untracked");

    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg("--lib")
        .arg(workspace_root().join("lib"))
        .arg(workspace_root().join("tools/viewer/refresh.loft"))
        .current_dir(&dir)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_NATIVE_LIBS", "1")
        .output()
        .expect("run the refresh driver");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the refresh driver failed: {stderr}\n{}",
        String::from_utf8_lossy(&out.stdout)
    );

    let read = |name: &str| -> String {
        std::fs::read_to_string(dir.join("tools/viewer/state").join(name))
            .unwrap_or_else(|e| panic!("no {name}: {e}"))
    };
    // Read the documents with a plain substring check rather than a JSON parser:
    // what is being tested is that the CONTENT came from this repository, and a
    // dependency on a parser here would only add a second thing that can be
    // wrong.
    let branch = read("branch.json");
    for want in [
        git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
        git(&dir, &["rev-parse", "--short", "HEAD"]),
    ] {
        assert!(
            branch.contains(&want),
            "branch.json does not carry {want:?}: {branch}"
        );
    }
    // The subject with an em-dash, quotes and a `%` — escaped as JSON, so the
    // check is on a distinctive fragment rather than the whole line.
    assert!(
        branch.contains("ünïcøde"),
        "branch.json lost the head subject: {branch}"
    );
    assert!(
        branch.contains("\"ahead\"") && branch.contains("\"behind\""),
        "branch.json is missing the ahead/behind counts: {branch}"
    );

    let commits = read("commits.json");
    for sha in git(&dir, &["log", "-20", "--pretty=%h"]).lines() {
        assert!(
            commits.contains(sha),
            "commits.json is missing {sha}: {commits}"
        );
        // Every listed commit has its diff and its per-file counts beside it —
        // the activity cards read both, and a missing one renders as an empty
        // card rather than an error.
        for name in [format!("{sha}.diff"), format!("{sha}.files.json")] {
            assert!(
                dir.join("tools/viewer/state/commits").join(&name).is_file(),
                "no {name} written"
            );
        }
    }
    // A tab in a commit subject survives into the document. This is the shape
    // the bash lost: it split `git log` output on TAB.
    assert!(
        commits.contains("first\\tcommit with a tab"),
        "the tab-bearing subject did not survive into commits.json: {commits}"
    );

    let uncommitted = read("uncommitted.json");
    assert!(
        uncommitted.contains("c.txt") && uncommitted.contains("??"),
        "uncommitted.json missed the untracked file: {uncommitted}"
    );
}

/// The program every "nothing to read" assertion below runs.
const NOTHING_TO_READ: &str = "fn main() {\n\
                               \x20   println(\"branch=[{branch()}]\");\n\
                               \x20   println(\"logn={len(log(5))}\");\n\
                               \x20   println(\"chgn={len(changed(\"main\"))}\");\n\
                               \x20   println(\"uncn={len(uncommitted())}\");\n\
                               \x20   println(\"hasmain={has_ref(\"main\")}\");\n\
                               \x20   println(\"showlen={size(show(\"deadbeef\"))}\");\n\
                               }\n";

/// In a repository with nothing in it yet — no commit, so no branch and no ref
/// that resolves — every query answers "nothing", rather than crashing or
/// reporting git's own text as data.
#[test]
fn a_question_with_no_answer_is_empty_not_a_failure() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skip: no git on this machine");
        return;
    }
    let dir = scratch("emptyrepo");
    Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .arg(&dir)
        .output()
        .expect("git init");
    for placement in ["inproc", "process"] {
        let (out, err) = run_loft(&dir, NOTHING_TO_READ, placement);
        for expect in [
            "branch=[]",
            "logn=0",
            "chgn=0",
            "hasmain=false",
            "showlen=0",
        ] {
            assert!(
                out.contains(expect),
                "expected {expect:?} under {placement}, got {out:?} / {err}"
            );
        }
        // The probe writes its own files into this directory, so git has
        // something to report here even though the history has nothing — and
        // git's own count is the only oracle that stays true either way.
        let untracked = git(&dir, &["status", "--porcelain"]).lines().count();
        assert!(
            out.contains(&format!("uncn={untracked}")),
            "expected uncn={untracked} under {placement}, got {out:?} / {err}"
        );
    }
}

/// A directory that is NOT a repository is a different question from a
/// repository with nothing to report, and loft#1061 is what happens when the two
/// answer the same: run outside a repository, every query answered empty and the
/// viewer rendered a repository with no branch, no commits and no files, with
/// nothing saying the question had never been asked. So this one must HALT, and
/// name the directory it looked in — an answer a caller can act on.
#[test]
fn outside_a_repository_is_a_failure_not_an_empty_answer() {
    let dir = scratch("norepo");
    for placement in ["inproc", "process"] {
        let (out, err) = run_loft(&dir, NOTHING_TO_READ, placement);
        assert!(
            err.contains("not a git repository"),
            "no diagnostic under {placement}: {out:?} / {err}"
        );
        assert!(
            err.contains(&dir.display().to_string()),
            "the diagnostic under {placement} does not name {}: {err}",
            dir.display()
        );
        // The empty answer must not reach the program at all — printing it and
        // then failing is the half-truth the viewer already acted on.
        assert!(
            !out.contains("branch=["),
            "the empty answer still reached the program under {placement}: {out:?}"
        );
    }
}
