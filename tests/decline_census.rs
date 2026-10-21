// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The decline census (`@FR-R-PushFill`, `src/rewrite_census.rs::declined`) — a rewrite's
//! refusals are counted beside its admissions, once per site, under the reason the site
//! names, and only for a shape the rule is for.  `scripts/decline_census.py` reads this
//! file to say what share of its shapes each rule reaches.
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One admitted push loop, one refused for a `break` (the body can leave early), and an
/// outer loop with no push of its own, which is no shape of the rule and must not count.
const PROGRAM: &str = "\
fn admitted(n: integer) -> integer {
  v: vector<integer> = [];
  for i in 0..n { v += [i * 3]; }
  len(v)
}

fn with_a_break(n: integer) -> integer {
  v: vector<integer> = [];
  for i in 0..n { if i > 3 { break; } v += [i]; }
  len(v)
}

fn outer(n: integer) -> integer {
  total = 0;
  for r in 0..n {
    v: vector<integer> = [];
    for i in 0..r { v += [i + r]; }
    total += len(v);
  }
  total
}

fn main() {
  println(\"{admitted(5)} {with_a_break(9)} {outer(4)}\");
}
";

fn loft() -> Command {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "120").env("LOFT_NO_CACHE", "1");
    cmd
}

/// `(rule, reason, count)` rows of the decline census.
type Declines = Vec<(String, String, u64)>;
/// `(rule, count)` rows of the rewrite census.
type Admissions = Vec<(String, u64)>;

fn census(dir: &Path) -> (Declines, Admissions) {
    let program = dir.join("probe.loft");
    fa::write(&program, PROGRAM).expect("write the probe");
    let (adm, dec, out) = (dir.join("adm.tsv"), dir.join("dec.tsv"), dir.join("out.rs"));
    let mut cmd = loft();
    cmd.arg("--native-emit")
        .arg(&out)
        .arg("--lean")
        .arg(&program)
        .env("LOFT_REWRITE_CENSUS", &adm)
        .env("LOFT_DECLINE_CENSUS", &dec);
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        status.status.success() && fa::exists(&dec),
        "no decline census written (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let declines = fa::read_to_string(&dec)
        .expect("read the declines")
        .lines()
        .map(|l| {
            let mut c = l.split('\t');
            (
                c.next().unwrap_or("").to_owned(),
                c.next().unwrap_or("").to_owned(),
                c.next().and_then(|n| n.parse().ok()).unwrap_or(0),
            )
        })
        .collect();
    let admissions = fa::read_to_string(&adm)
        .expect("read the admissions")
        .lines()
        .map(|l| {
            let (rule, n) = l.split_once('\t').unwrap_or((l, "0"));
            (rule.to_owned(), n.parse().unwrap_or(0))
        })
        .collect();
    (declines, admissions)
}

#[test]
fn a_refused_push_loop_is_one_decline_under_its_reason_and_an_outer_loop_is_none() {
    let dir = std::env::temp_dir().join(format!("loft-decline-census-{}", std::process::id()));
    fa::create_dir_all(&dir).expect("temp dir");
    let (declines, admissions) = census(&dir);
    let _ = fa::remove_dir_all(&dir);
    let push: Vec<_> = declines
        .iter()
        .filter(|(rule, _, _)| rule == "R-PushFill/loop")
        .collect();
    assert_eq!(
        push,
        vec![&(
            "R-PushFill/loop".to_owned(),
            "the body can leave early, or loops".to_owned(),
            1
        )],
        "one refused loop, one decline, under the reason the trace names — the outer loop of \
         `outer`, which pushes nothing itself, is not a shape of the rule: {declines:?}"
    );
    // Three loops take a window: `admitted`'s, `outer`'s inner one (its two emitted copies
    // are one site), and `with_a_break`'s — refused the counted form, since its trip count
    // is not known before it runs, it still takes the DEEP window, which needs no count.
    // The decline census counts the LOOP once (`rule<TAB>admitted<TAB>n`); the rewrite
    // census counts every emitted copy, so its figure is never smaller.
    let windows = declines
        .iter()
        .find(|(rule, why, _)| rule == "R-PushFill" && why == "admitted")
        .map_or(0, |(_, _, n)| *n);
    assert_eq!(
        windows, 3,
        "three push loops admitted a window: {declines:?}"
    );
    let copies = admissions
        .iter()
        .find(|(rule, _)| rule == "R-PushFill")
        .map_or(0, |(_, n)| *n);
    assert!(
        copies >= windows,
        "the rewrite census counts every copy: {admissions:?}"
    );
    assert!(
        !declines
            .iter()
            .any(|(rule, why, _)| rule == "R-PushFill" && why != "admitted"),
        "no window refused in this program: {declines:?}"
    );
}
