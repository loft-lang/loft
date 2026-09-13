// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `LOFT_NATIVE_CHECKPOINTS` — the per-OPERATOR instrument the generator writes into the
//! emitted Rust, for the cases a sampler cannot reach (a stripped binary, a box without
//! `perf`, wasm).  Three things are pinned here, because each one is a way the instrument
//! could be quietly useless:
//!
//! 1. **Off is off.**  The default emission carries no checkpoint text at all, so an
//!    ordinary build cannot pay for a switch nobody asked for.
//! 2. **The counts are RIGHT.**  A loop whose trip count is known by hand must report that
//!    number — not one more, not one fewer.  An instrument that merely produces plausible
//!    numbers is the failure mode worth testing for, and the off-by-one between a loop's
//!    BODY and its counter is exactly where a misplaced probe shows up.
//! 3. **The site is attributed to the right loft function**, which is the whole reason the
//!    instrument exists beside `perf`: an optimised build inlines callees into their
//!    caller, and a checkpoint is keyed to the function the operator was WRITTEN in.
use std::path::PathBuf;
use std::process::Command;

/// A loop with a hand-computable trip count: the body runs exactly `N` times, and the
/// loop's own counter and test run `N + 1`.
const SRC: &str = r#"
fn work(n: integer) -> integer {
  w_t = 0;
  for i in 0..n { w_t = w_t + i * 3; }
  w_t
}
fn main() { print("t {work(1000)}") }
"#;

fn write_src(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("ckpt_probe.loft");
    std::fs::write(&p, SRC).expect("write probe");
    p
}

fn loft() -> Command {
    let mut c = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    c.env("LOFT_TIMEOUT", "180");
    c
}

#[test]
fn default_emission_carries_no_checkpoint() {
    let dir = std::env::temp_dir().join(format!("loft_probe_off_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let src = write_src(&dir);
    let out = dir.join("off.rs");
    let o = loft()
        .arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .output()
        .expect("emit");
    assert!(out.exists(), "no Rust emitted: {:?}", o.status);
    let rust = std::fs::read_to_string(&out).expect("read");
    // Match the MACHINERY, not the bare token: the emitted `// loft:<path>` comments
    // carry the probe file's own directory, so a scratch path mentioning the instrument
    // would read as a hit.  (It did — that is why this looks for the macro call.)
    let hits: Vec<&str> = rust
        .lines()
        .filter(|l| {
            l.contains("loft_ckpt_c!") || l.contains("loft_ckpt_t!") || l.contains("LOFT_CKPT_")
        })
        .take(3)
        .collect();
    assert!(
        hits.is_empty(),
        "an uninstrumented build must carry no checkpoint machinery; found {} bytes, exit {:?}, first hits: {hits:?}",
        rust.len(),
        o.status
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn count_mode_counts_the_loop_exactly() {
    let dir = std::env::temp_dir().join(format!("loft_probe_count_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let src = write_src(&dir);
    let o = loft()
        .arg("--native")
        .arg(&src)
        .env("LOFT_NATIVE_CHECKPOINTS", "count")
        .output()
        .expect("run");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        all.contains("loft checkpoints"),
        "no checkpoint report:\n{all}"
    );
    // The program's own answer must be untouched by the instrument.
    assert!(all.contains("t 1498500"), "wrong result:\n{all}");

    let row = |op: &str| -> u64 {
        all.lines()
            .find(|l| l.contains(op) && l.contains("n_work"))
            .and_then(|l| l.split_whitespace().nth(1).and_then(|n| n.parse().ok()))
            .unwrap_or_else(|| panic!("no {op} row for n_work in:\n{all}"))
    };
    // `i * 3` is in the BODY: exactly one per iteration.
    assert_eq!(row("OpMulInt"), 1000, "the multiply is one per iteration");
    // The loop TEST runs once more than the body — the last one is what ends the loop.
    assert_eq!(row("OpLeInt"), 1001, "the loop test runs N+1 times");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sites_name_the_loft_function_the_operator_was_written_in() {
    let dir = std::env::temp_dir().join(format!("loft_probe_site_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let src = write_src(&dir);
    let o = loft()
        .arg("--native")
        .arg(&src)
        .env("LOFT_NATIVE_CHECKPOINTS", "count")
        .output()
        .expect("run");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    // `work`'s arithmetic is attributed to `n_work` at the line it was written on, even
    // though `main` is what calls it — the property `perf` loses to inlining.
    let mul = all
        .lines()
        .find(|l| l.contains("OpMulInt"))
        .unwrap_or_else(|| panic!("no OpMulInt row:\n{all}"));
    assert!(mul.contains("n_work"), "wrong owner: {mul}");
    assert!(
        mul.contains("ckpt_probe.loft:4"),
        "wrong source line: {mul}"
    );
    assert!(all.contains("-- by function --"), "no rollup:\n{all}");
    let _ = std::fs::remove_dir_all(&dir);
}
