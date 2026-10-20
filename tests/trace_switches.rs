// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! A trace switch REPORTS and never changes what compiles: every `LOFT_TRACE_*` named in
//! `src/` at once, against none, must print the same program output on both backends.  A
//! trace line that dereferences a context the program does not have (the fuse trace named
//! `self.context` inside a constant's initialiser, outside any function) is an internal
//! compiler error under the switch and invisible without it.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const PROGRAM: &str = r#"
NAMES: vector<text> = ["a", "bb", "ccc"];
enum Kind { None, Raise }
struct St { k: Kind, n: integer, xs: vector<integer> }
fn mk(n: integer) -> St {
  s = St { k: Kind.None, n: n, xs: [] };
  for i in 0..n { s.xs += [i * i]; }
  s
}
fn main() {
  s = mk(4);
  t = 0;
  for x in s.xs { t += x; }
  w = "";
  for nm in NAMES { w += nm; }
  println("{t} {len(NAMES)} {w}");
}
"#;

fn trace_switches() -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        for e in fa::read_dir(&dir).expect("read src") {
            let p = e.os_spelling();
            if fa::is_dir(&p) {
                stack.push(p);
            } else if fa::has_extension(&p, "rs") {
                let src = fa::read_to_string(&p).unwrap_or_default();
                for part in src.split("\"LOFT_TRACE_").skip(1) {
                    let name: String = part
                        .chars()
                        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                        .collect();
                    if part[name.len()..].starts_with('"') && !name.is_empty() {
                        names.insert(format!("LOFT_TRACE_{name}"));
                    }
                }
            }
        }
    }
    names.into_iter().collect()
}

fn run(backend: &str, file: &Path, traces: &[String]) -> (String, String, bool) {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg(backend)
        .arg(file)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NATIVE_NO_CACHE", "1");
    for t in traces {
        cmd.env(t, "1");
    }
    let out = cmd.output().expect("spawn loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn every_trace_switch_at_once_leaves_the_program_unchanged() {
    let traces = trace_switches();
    assert!(
        traces.len() > 50,
        "found only {} trace switches",
        traces.len()
    );
    let file =
        std::env::temp_dir().join(format!("loft_trace_switches_{}.loft", std::process::id()));
    fa::write(&file, PROGRAM).expect("write program");
    for backend in ["--interpret", "--native"] {
        let (plain, err, ok) = run(backend, &file, &[]);
        assert!(
            ok && plain.contains("14 3 abbccc"),
            "{backend} without traces:\n{plain}\n{err}"
        );
        let (traced, err, ok) = run(backend, &file, &traces);
        let tail: String = err.lines().rev().take(20).collect::<Vec<_>>().join("\n");
        assert!(
            ok && traced == plain,
            "{backend} with every trace switch:\n{traced}\n{tail}"
        );
    }
    let _ = fa::remove_file(&file);
}
