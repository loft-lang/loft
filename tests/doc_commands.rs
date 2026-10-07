// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Every command a doc page SHOWS is run, and its shown output checked.
//!
//! The language examples in `tests/docs/*.loft` have always been executed — the pages say
//! so ("each page is also a live test"), and that is why a reader can trust them. The
//! pages that teach how to USE loft are shell transcripts, and prose is exactly what rots:
//! a renamed flag or a reworded report leaves the documentation confidently wrong with
//! nothing failing.
//!
//! The rule is one character. In a doc page's prose, an indented line starting with `$ `
//! is a COMMAND: it is executed here, and the indented lines under it are expected output,
//! each of which must appear in what the command printed. An indented line without `$ ` is
//! illustration and is never run — which is how a page can still show an interactive
//! session (`loft` on its own) that cannot be driven from a script.
//!
//! Commands run through `sh -c` in a copy of `tests/docs/cli/`, with the freshly built
//! `loft` first on `PATH`. So a transcript is what a reader would actually type — pipes,
//! `cd`, and all — rather than a shape invented for the test.

use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// One `$` line plus the output lines under it.
struct Transcript {
    page: String,
    line_no: usize,
    command: String,
    expected: Vec<String>,
}

/// Strip a doc-comment prefix, returning the prose line's content, or `None` for code.
fn prose(line: &str) -> Option<&str> {
    line.strip_prefix("// ").or_else(|| line.strip_prefix("//"))
}

/// Collect every `$` transcript in one page.
///
/// An expected-output line ends the block when it stops being indented, so a block is
/// exactly what a reader sees as one screenful.
fn transcripts_in(page: &Path) -> Vec<Transcript> {
    let text = fa::read_to_string(page).expect("read doc page");
    let name = page.file_name().unwrap().to_string_lossy().into_owned();
    let mut out: Vec<Transcript> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let Some(body) = prose(raw) else { continue };
        let Some(indented) = body.strip_prefix("    ") else {
            continue;
        };
        if let Some(cmd) = indented.trim_start().strip_prefix("$ ") {
            out.push(Transcript {
                page: name.clone(),
                line_no: i + 1,
                command: cmd.trim().to_string(),
                expected: Vec::new(),
            });
        } else if let Some(last) = out.last_mut() {
            // Only lines directly under the command, before any blank prose line.
            if last.line_no + last.expected.len() + 1 == i + 1 && !indented.trim().is_empty() {
                last.expected.push(indented.trim().to_string());
            }
        }
    }
    out
}

/// A writable copy of the fixture tree, so a command that writes (a `.loft/` cache, a
/// built binary) cannot touch the repository.
fn fixture_copy(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("loft_doccmd_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    copy_tree(Path::new("tests/docs/cli"), &root);
    root
}

fn copy_tree(from: &Path, to: &Path) {
    fa::create_dir_all(to).expect("mkdir fixture");
    for entry in std::fs::read_dir(from).expect("read fixture dir") {
        let entry = entry.expect("fixture entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fa::copy(entry.path(), &target).expect("copy fixture file");
        }
    }
}

#[test]
fn every_documented_command_runs_and_prints_what_the_page_shows() {
    let pages: Vec<PathBuf> = std::fs::read_dir("tests/docs")
        .expect("read tests/docs")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "loft"))
        .collect();

    let mut all: Vec<Transcript> = Vec::new();
    for page in &pages {
        all.extend(transcripts_in(page));
    }
    assert!(
        !all.is_empty(),
        "no `$` transcripts found — the marker changed, or the pages lost their examples"
    );

    let root = fixture_copy("run");
    let bin_dir = loft_bin().parent().expect("bin dir").to_path_buf();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let mut failures: Vec<String> = Vec::new();
    for t in &all {
        let out = Command::new("sh")
            .arg("-c")
            .arg(&t.command)
            .current_dir(&root)
            .env("PATH", &path)
            .env("LOFT_TIMEOUT", "120")
            .output()
            .unwrap_or_else(|e| panic!("could not run `{}`: {e}", t.command));
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if !out.status.success() {
            failures.push(format!(
                "{}:{} `{}` exited {:?}\n{combined}",
                t.page,
                t.line_no,
                t.command,
                out.status.code()
            ));
            continue;
        }
        for want in &t.expected {
            if !combined.contains(want.as_str()) {
                failures.push(format!(
                    "{}:{} `{}` did not print `{want}`\n--- it printed ---\n{combined}",
                    t.page, t.line_no, t.command
                ));
            }
        }
    }
    let _ = fa::remove_dir_all(&root);
    assert!(
        failures.is_empty(),
        "{} of {} documented commands did not behave as the page shows:\n\n{}",
        failures.len(),
        all.len(),
        failures.join("\n\n")
    );
}
