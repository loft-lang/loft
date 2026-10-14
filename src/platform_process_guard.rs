// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! The rule's guard (@PLN184 P6): every process loft starts goes through `platform::process`.
//!
//! Clippy refuses `std::process::Command::new` everywhere (`clippy.toml`); this module is what
//! keeps the exceptions honest.  Every `Command::new` spelled outside `platform::process` — in
//! `src/`, `tests/` (minus the library fixtures, crates of their own) or `build.rs` — is counted
//! per file against `platform_process_clippy_allow.baseline`, and each such file must carry the
//! marked opt-out (`#![allow(clippy::disallowed_methods, …)]`) Clippy needs to pass it, so the
//! baseline IS the opt-out list.  A count that RISES fails; one that FALLS fails too, until it
//! is locked in — `LOFT_BLESS_PLATFORM_PROCESS=1 cargo test --lib platform::process::guard` —
//! so the list only shrinks.  And the test harnesses' raw constructor,
//! `platform::process::harness_command`, appears nowhere in `src/`.

use crate::file_access::{self, PathText};
use std::collections::BTreeMap;
use std::fmt::Write as _;

const BASELINE: &str = "src/platform_process_clippy_allow.baseline";

/// The one home of `Command::new`, and this guard, which spells it in its own tests.
const HOME: &[&str] = &["src/platform_process.rs", "src/platform_process_guard.rs"];

/// The marked opt-out a file needs for Clippy to pass a `Command::new` in it.
const OPT_OUT: &str = "#![allow(clippy::disallowed_methods";

/// Occurrences of `pat` in this line's code: not in a comment line, not in a string literal.
fn spelled(line: &str, pat: &str) -> usize {
    if line.trim_start().starts_with("//") {
        return 0;
    }
    line.match_indices(pat)
        .filter(|(at, _)| {
            // Not the tail of a longer name (`MyCommand::new`).
            let b = line.as_bytes();
            let whole = *at == 0 || !(b[at - 1].is_ascii_alphanumeric() || b[at - 1] == b'_');
            whole && !file_access::guard::in_string(line, *at)
        })
        .count()
}

fn is_rust(p: &PathText) -> bool {
    p.parts()
        .last()
        .and_then(|n| n.rsplit_once('.'))
        .is_some_and(|(stem, e)| !stem.is_empty() && e == "rs")
}

/// `pat` occurrences per file under `dirs` (and the single files among them), outside
/// [`HOME`], the library fixtures and generated output.
fn measure(root: &PathText, dirs: &[&str], pat: &str) -> BTreeMap<String, (usize, bool)> {
    let mut out = BTreeMap::new();
    let mut stack: Vec<PathText> = dirs.iter().map(|d| root.join(d)).collect();
    while let Some(p) = stack.pop() {
        let rel = p
            .relative_to(root)
            .map(|r| r.portable())
            .unwrap_or_default();
        if HOME.contains(&rel.as_str())
            || rel == "tests/fixtures"
            || rel
                .split('/')
                .any(|c| matches!(c, ".loft" | "target" | "node_modules"))
        {
            continue;
        }
        if file_access::is_dir(&p) {
            stack.extend(file_access::read_dir(&p).unwrap_or_default());
        } else if is_rust(&p)
            && let Ok(text) = file_access::read_to_string(&p)
        {
            let n: usize = text.lines().map(|l| spelled(l, pat)).sum();
            if n > 0 {
                let marked = text.lines().any(|l| l.starts_with(OPT_OUT));
                out.insert(rel, (n, marked));
            }
        }
    }
    out
}

fn read_baseline(path: &PathText) -> BTreeMap<String, usize> {
    file_access::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (n, file) = l.split_once(' ')?;
            Some((file.to_string(), n.parse().ok()?))
        })
        .collect()
}

fn write_baseline(path: &PathText, now: &BTreeMap<String, usize>) {
    let mut out = String::from(
        "# Files still spelling std::process::Command::new outside platform::process, each with a\n\
         # marked #![allow(clippy::disallowed_methods, …)] (@PLN184 P6).\n\
         # Only shrinks: see src/platform_process_guard.rs.  Format: <count> <file>\n",
    );
    for (f, n) in now {
        let _ = writeln!(out, "{n} {f}");
    }
    file_access::write(path, out).expect("write baseline");
}

#[test]
fn every_process_goes_through_platform_process() {
    let root = PathText::host(env!("CARGO_MANIFEST_DIR"));
    let found = measure(&root, &["src", "tests", "build.rs"], "Command::new(");
    let unmarked: Vec<&String> = found
        .iter()
        .filter(|(_, (_, marked))| !marked)
        .map(|(f, _)| f)
        .collect();
    assert!(
        unmarked.is_empty(),
        "these spell `Command::new` without the marked opt-out Clippy needs — start the \
         process through `platform::process::Spawn` instead (src/platform_process.rs), or in a \
         test harness through `platform::process::harness_command`:\n  {unmarked:?}"
    );
    let now: BTreeMap<String, usize> = found.into_iter().map(|(f, (n, _))| (f, n)).collect();
    let base_path = root.join(BASELINE);
    let bless = std::env::var_os("LOFT_BLESS_PLATFORM_PROCESS").is_some();
    // Bless writes a missing baseline as it finds the tree; a rise is never blessed.
    if bless && !file_access::exists(&base_path) {
        write_baseline(&base_path, &now);
        return;
    }
    let base = read_baseline(&base_path);
    let rose: Vec<String> = now
        .iter()
        .filter(|(f, n)| **n > base.get(*f).copied().unwrap_or(0))
        .map(|(f, n)| format!("{f}: {} -> {n}", base.get(f).copied().unwrap_or(0)))
        .collect();
    assert!(
        rose.is_empty(),
        "`Command::new` ROSE outside `platform::process` — start the process through \
         `platform::process::Spawn` (src/platform_process.rs):\n  {}",
        rose.join("\n  ")
    );
    let fell: Vec<String> = base
        .iter()
        .filter(|(f, n)| now.get(*f).copied().unwrap_or(0) < **n)
        .map(|(f, n)| format!("{f}: {n} -> {}", now.get(f).copied().unwrap_or(0)))
        .collect();
    if bless {
        write_baseline(&base_path, &now);
        return;
    }
    assert!(
        fell.is_empty(),
        "`Command::new` FELL outside `platform::process` — lock it in: \
         LOFT_BLESS_PLATFORM_PROCESS=1 cargo test --lib platform::process::guard\n  {}",
        fell.join("\n  ")
    );
}

/// The raw constructor is for a test harness; loft's own code starts a process through
/// `Spawn`, which owns its tree.
#[test]
fn harness_command_stays_out_of_src() {
    let root = PathText::host(env!("CARGO_MANIFEST_DIR"));
    let found = measure(&root, &["src"], "harness_command(");
    assert!(
        found.is_empty(),
        "`platform::process::harness_command` is for tests/ — start the process through \
         `platform::process::Spawn` instead:\n  {:?}",
        found.keys().collect::<Vec<_>>()
    );
}

#[test]
fn the_guard_sees_a_spawn_and_nothing_else() {
    assert_eq!(
        spelled("    let c = Command::new(\"git\");", "Command::new("),
        1
    );
    assert_eq!(
        spelled(
            "    std::process::Command::new(x).output()",
            "Command::new("
        ),
        1
    );
    assert_eq!(
        spelled("    // Command::new(\"git\") here", "Command::new("),
        0
    );
    assert_eq!(spelled("    /// `Command::new(p)`", "Command::new("), 0);
    assert_eq!(
        spelled("    out.push_str(\"Command::new(x)\");", "Command::new("),
        0
    );
    assert_eq!(spelled("    MyCommand::new(x)", "Command::new("), 0);
    assert_eq!(spelled("    harness_command(exe)", "harness_command("), 1);
}
