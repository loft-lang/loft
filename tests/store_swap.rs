// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN203 rust-writer: counts the Rust writer's store exchanges — its `--native` runs as `--native-rust` until then.
//! `@FR-H-SwapIn` — the rebind copy of a minted result into a reset root is an exchange of the
//! two stores, PINNED on the cells (`tests/scripts/a-rebind-from-a-fresh-result-exchanges-the-
//! stores.loft`) by counting the exchanges `LOFT_TRACE_STORE_SWAP=1` names: the cells' values
//! pass on either form, so only this count sees the rule stop firing — on either backend, and
//! not at all under the switch.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-rebind-from-a-fresh-result-exchanges-the-stores.loft";

/// The exchanges one run of the cells makes, and its output.
fn swaps(mode: &str, env: &[(&str, &str)]) -> (usize, String) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg(mode)
        .arg(&src)
        .env("LOFT_TIMEOUT", "240")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TRACE_STORE_SWAP", "1")
        // `@FR-R-RebindBuffer` takes s12's rebind loop on `--native` by adopting instead of
        // exchanging, and `@FR-O-Move`'s rebind adopt (`bind_lowering::re_bind`) takes it on the
        // interpreter; this pins the exchange itself, so it is taken with both rules off.
        .env("LOFT_NO_REBIND_BUFFER", "1")
        .env("LOFT_NO_REBIND_ADOPTS_MINTED", "1")
        .env_remove("LOFT_NO_STORE_SWAP");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    (
        stderr
            .lines()
            .filter(|l| l.starts_with("[swap-in]"))
            .count(),
        stdout,
    )
}

/// s12's 100 000 rebinds are the floor: each is one exchange on both backends.
#[test]
fn the_rebinds_exchange_their_stores_on_both_backends() {
    for mode in ["--interpret", "--native-rust"] {
        let (n, out) = swaps(mode, &[]);
        assert!(
            out.contains("done"),
            "{mode}: the cells did not finish:\n{out}"
        );
        assert!(
            n >= 100_000,
            "{mode}: {n} exchanges — the rule stopped firing"
        );
    }
}

#[test]
fn the_switch_keeps_every_copy() {
    for mode in ["--interpret", "--native-rust"] {
        let (n, out) = swaps(mode, &[("LOFT_NO_STORE_SWAP", "1")]);
        assert!(
            out.contains("done"),
            "{mode}: the cells did not finish:\n{out}"
        );
        assert_eq!(n, 0, "{mode}: LOFT_NO_STORE_SWAP=1 must keep every copy");
    }
}

/// The three rebinds `c = mk(3, …)`, from a callee that mints its hash, each exchange the
/// stores; `WALK` makes them inside a walk of a hash (`for e in w`, a live scratch) and `AFTER`
/// right after it.  The hash's last version is `mk(3, 20)`: 20 + 21 + 22 = 63, three entries.
const GATE: &str = "struct E { k: integer, v: integer }
fn mk(n: integer, base: integer) -> hash<E[k]> {
  h: hash<E[k]> = [];
  for i in 0..n { h += [E { k: i, v: base + i }]; }
  h
}
fn sumh(h: hash<E[k]>) -> integer { t = 0; for e in h { t += e.v; } t }
fn main() {
  c = mk(1, 0);
  w = mk(3, 0);
  REBINDS
  println(\"sum {sumh(c)} len {len(c)}\");
}
";

/// The exchanges one run of `GATE` with `rebinds` makes, and its output.
fn gate_swaps(mode: &str, tag: &str, rebinds: &str) -> (usize, String) {
    let dir = std::env::temp_dir().join(format!("loft_swap_gate_{}", std::process::id()));
    loft::file_access::create_dir_all(&dir).expect("probe dir");
    let src = dir.join(format!("{tag}_{}.loft", &mode[2..]));
    loft::file_access::write(&src, GATE.replace("REBINDS", rebinds)).expect("write probe");
    let out = loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg(mode)
        .arg(&src)
        .env("LOFT_TIMEOUT", "240")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TRACE_STORE_SWAP", "1")
        .env_remove("LOFT_NO_STORE_SWAP")
        .output()
        .expect("spawn loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    (
        stderr
            .lines()
            .filter(|l| l.starts_with("[swap-in]"))
            .count(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// `@FR-H-SwapIn`'s gates (STORE_MODEL D8): no exchange while a walk scratch is live, on both
/// backends.  The same three rebinds exchange right after the walk and copy inside it, and
/// answer the same either way.
///
/// @falsified-at: none — the gate predates the guard, so the receipt is a SABOTAGE (2026-11-21):
///   with the scratch term struck from both gates (`swap_stores_in_as`'s
///   `!self.live_scratches.is_empty()`, `loft_swap_blocked`'s `loft_nscratches != 0`), the walk
///   run exchanges as the after run does and the difference reads 0, not 3, on both backends.
///   CHANNEL: the `[swap-in]` count.  WITNESS: 5 and 5 instead of 5 and 2.
///   HOLDS: `sum 63 len 3` on every run, gate or no gate.
#[test]
fn no_store_is_exchanged_while_a_walk_is_live_on_both_backends() {
    for mode in ["--interpret", "--native"] {
        let (walk, wout) = gate_swaps(mode, "walk", "for e in w { c = mk(3, e.k * 10); }");
        let (after, aout) = gate_swaps(
            mode,
            "after",
            "for e in w { t = e.k; }\n  for r in 0..3 { c = mk(3, r * 10); }",
        );
        assert!(wout.contains("sum 63 len 3"), "{mode} walk: {wout}");
        assert!(aout.contains("sum 63 len 3"), "{mode} after: {aout}");
        assert_eq!(
            after - walk,
            3,
            "{mode}: the three rebinds exchange after the walk ({after}) and copy inside it ({walk})"
        );
    }
}
