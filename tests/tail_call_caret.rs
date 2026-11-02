// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-TailSelf` — `warning[tail-call-not-jumped]` names the ARGUMENT that keeps the call an
//! ordinary call, so it stands on that argument.  It stood on the call's IR position, the token
//! after the `(`, which falls inside the first argument only: an offending second argument was
//! reported on the first (`deepest(depth + 1, k[0])` pointed at the `+`).

use loft::file_access as fa;

const PROGRAM: &str = "struct N { kids: vector<N>, v: integer }
fn deepest(depth: integer, n: N) -> integer {
  if len(n.kids) == 0 {
    return depth;
  }
  k = n.kids;
  deepest(depth + 1, k[0])
}
fn main() { t = N { kids: [N { kids: [], v: 2 }], v: 1 }; println(\"{deepest(0, t)}\"); }
";

#[test]
fn the_warning_stands_on_the_argument_it_names() {
    let dir = std::env::temp_dir().join(format!("loft-tail-caret-{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join("tail.loft");
    fa::write(&file, PROGRAM).expect("write probe");
    let out = loft::platform::process::harness_command(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_loft"
    )))
    .arg("--interpret")
    .arg(&file)
    .env("LOFT_TIMEOUT", "60")
    .env("LOFT_NO_CACHE", "1")
    .output()
    .expect("failed to invoke loft");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "the probe must run:\n{err}");
    assert!(
        err.contains("argument for parameter `n`"),
        "the warning names the argument for `n`:\n{err}"
    );
    let name = file.to_string_lossy().to_string();
    let at = err
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("--> "))
        .find_map(|a| {
            a.strip_prefix(name.as_str())?
                .strip_prefix(':')
                .map(str::to_string)
        })
        .unwrap_or_else(|| panic!("no position printed:\n{err}"));
    let (line, col) = at.split_once(':').expect("line:col");
    let line: usize = line.parse().expect("line");
    let col: usize = col.parse().expect("col");
    let text = PROGRAM.lines().nth(line - 1).expect("the line");
    let start = text.find("k[0]").expect("the argument") + 1;
    assert!(
        line == 7 && (start..start + 4).contains(&col),
        "the caret is at {line}:{col}; it belongs inside `k[0]` (line 7, columns {start}..{}):\n{err}",
        start + 3
    );
}
