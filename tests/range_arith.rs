// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Range` — the EMISSION pins.  An integer operator whose RESULT provably fits the
//! type emits the processor's operator (`wrapping_add`/`wrapping_sub`/`wrapping_mul`/
//! `wrapping_neg`, a bare `/` or `%`); one whose range the proof cannot bound keeps its
//! checked template (`ops::op_add_int(` and kin, the `*Nullable` twins, the `_long_nn`
//! forms).  `LOFT_NO_RANGE_ARITH=1` restores every template.  The cell corpus
//! (`tests/scripts/157-range-arith.loft`) says the VALUES hold on both backends, in every
//! switch state and under the falsifiers; this pins what is emitted.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/157-range-arith.loft";

/// Per function: `(wrapping add, sub, mul, neg; checked `op_add_int(`, `op_mul_int(`;
/// plain `/`, plain `%`; checked nullable divisions; checked nullable multiplies)` — written
/// beside the cells, then checked against the emission.
const EXPECTED: &[(&str, [usize; 10])] = &[
    // a1: the composite chain — masks, `255 - sa`, the byte products and sums, `oa / 2`
    // all plain; the one division by the chain's own sum stays checked (its range holds 0).
    ("n_a1", [3, 1, 5, 0, 0, 0, 1, 0, 1, 0]),
    // a2: `m * m` (bound 2^62) and `m * 3` plain; `* 4` (bound 2^64) stays the checked
    // nullable twin; the three `-1` literals are plain negations.
    ("n_a2", [0, 0, 2, 3, 0, 0, 0, 0, 0, 1]),
    // a3: the product of two 31-bit masks fits — plain.
    ("n_a3", [0, 0, 1, 0, 0, 0, 0, 0, 0, 0]),
    // a4: `i * i` over the literal-ended counter is plain, and the counter's own `+ 1` step
    // with it (its index is ranged -1..=1000); `s + …` self-steps `s`, so both of ITS adds
    // stay checked.  This row is the one that exposed loft#1558: until 2026-09-21 the
    // `wrapping_mul` here came from the CHAIN GUARD's plain copy, not from the range proof,
    // because the counted-counter clause was inert — it looked for the counter's SEED inside
    // the loop and the parser emits it as the statement BEFORE.  The profitability gate took
    // the guard away and the borrowed evidence with it, which is how the gap surfaced.
    ("n_a4", [1, 0, 1, 0, 2, 0, 0, 0, 0, 0]),
    // a5: a parameter is not trusted — both operators keep their templates.
    ("n_a5_f", [0, 0, 0, 0, 1, 1, 0, 0, 0, 0]),
    // a6: the one-expression callees over a non-sentinel argument range: all plain.
    ("n_a6", [2, 0, 1, 0, 0, 0, 0, 0, 0, 0]),
    // a7: negation, the signed shift, `/` and `%` by a ranged divisor — plain.
    ("n_a7", [2, 1, 0, 3, 0, 0, 1, 1, 0, 0]),
    // a8: `len + size` fits (plain), `len * size` does not (checked).
    ("n_a8", [1, 0, 0, 0, 0, 1, 0, 0, 0, 0]),
    // a9: the mask of a null keeps its template — the mask rule needs a non-sentinel operand.
    ("n_a9", [0, 0, 0, 0, 1, 0, 0, 0, 0, 0]),
    // ── the counted-range counter clause (loft#1558) ──
    // b1: `i` is 0..=9, so `i * i` is plain and so is the counter's own step; `acc + …`
    // self-steps and stays checked.
    ("n_b1", [1, 0, 1, 0, 1, 0, 0, 0, 0, 0]),
    // b2: the same for the INCLUSIVE form, whose end is the counter's top rather than its
    // top minus one.
    ("n_b2", [1, 0, 1, 0, 1, 0, 0, 0, 0, 0]),
    // b3/b4 are the BOUNDARY PAIR and differ only in the range's end, so the multiply's
    // verdict is a claim about the counter's top bound and nothing else.  b3 reaches 10,
    // `10 * 1e18` is 1e19 and does not fit, so the multiply DECLINES to its nullable twin —
    // the plain column is the counter step alone.
    ("n_b3", [1, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
    // b4 stops at 9, `9 * 1e18` is exactly 9e18 and fits, so the multiply is ADMITTED.
    ("n_b4", [1, 0, 1, 0, 0, 0, 0, 0, 0, 0]),
    // The TYPE clause (loft#1593's other half): declared `u8` / `limit` parameters and the
    // compiler-typed join are ranged; a plain-integer index, an `i32`, a nullable and a boxed
    // capture are not.
    //
    // ⚠ A range that leaves codes UNUSED inside its width used to be in that second list, on
    // the reasoning that its overflow wrote a sentinel into the slot.  C127 retired that
    // sentinel — such a slot takes the type's DEFAULT, which is in range — so the declared
    // range is again exactly what the slot can hold and c8's row below flipped from checked to
    // plain.  That flip IS the C127 fact on the emission side, and it is why this row is the
    // one to read first if the ruling is ever narrowed.
    ("n_c1_join", [1, 0, 1, 0, 0, 0, 0, 0, 0, 0]),
    ("n_c2_head", [1, 0, 1, 0, 0, 0, 0, 0, 0, 0]),
    ("n_c3_read", [1, 0, 1, 0, 1, 0, 0, 0, 0, 0]),
    ("n_c4", [2, 0, 3, 0, 0, 0, 0, 0, 0, 0]),
    ("n_c5_i32", [0, 0, 0, 0, 1, 1, 0, 0, 0, 0]),
    ("n_c5_opt", [0, 0, 0, 0, 1, 0, 0, 0, 0, 0]),
    ("n_c5", [0, 0, 0, 0, 2, 0, 0, 0, 0, 0]),
    ("n_c6", [1, 0, 1, 0, 0, 0, 0, 0, 0, 0]),
    ("n_c7", [0, 0, 0, 0, 2, 2, 0, 0, 0, 2]),
    // c8: three declared ranges that leave codes spare — two signed byte/short aliases and
    // `limit(1000, 1100)`, whose range excludes zero.  Every step and every read after it is
    // PLAIN since C127; the one `wrapping_neg` is the negative literal in `y -= 1`.
    ("n_c8", [5, 1, 2, 1, 0, 0, 0, 0, 0, 0]),
    // b5: an end that is a `size`, ranged at seed time — `k + 1` is plain beside the
    // counter's step (two), while `acc + …` self-steps and stays checked.
    ("n_b5", [2, 0, 0, 0, 1, 0, 0, 0, 0, 0]),
    // b6: a NESTED loop — both counters are seeded (two plain steps) and `a * b` is plain.
    ("n_b6", [2, 0, 1, 0, 1, 0, 0, 0, 0, 0]),
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_RANGE_ARITH")
        .env_remove("LOFT_NO_GUARDED_CHAIN")
        .env_remove("LOFT_NO_NN_FAST")
        .env_remove("LOFT_HOIST_VERIFY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let out =
        std::env::temp_dir().join(format!("loft_range_arith_{}_{tag}.rs", std::process::id()));
    let status = loft(
        &[
            "--native-emit",
            out.to_str().unwrap(),
            cells().to_str().unwrap(),
        ],
        env,
    );
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = std::fs::read_to_string(&out).expect("read the emitted Rust");
    let _ = std::fs::remove_file(&out);
    rust
}

const KEYS: [&str; 10] = [
    "wrapping_add",
    "wrapping_sub",
    "wrapping_mul",
    "wrapping_neg",
    "ops::op_add_int(",
    "ops::op_mul_int(",
    ") / (",
    ") % (",
    "op_div_int_nullable",
    "op_mul_int_nullable",
];

fn counts(rust: &str) -> HashMap<String, [usize; 10]> {
    let mut map: HashMap<String, [usize; 10]> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        let row = map.entry(current.clone()).or_default();
        for (i, k) in KEYS.iter().enumerate() {
            row[i] += line.matches(k).count();
        }
    }
    map
}

fn run(args: &[&str], env: &[(&str, &str)]) -> String {
    let mut a: Vec<&str> = args.to_vec();
    let s = cells().to_str().unwrap().to_string();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.trim_end().ends_with("range arith ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn the_cells_answer_the_interpreter_in_every_switch_state_under_the_falsifiers() {
    let oracle = run(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [
        None,
        Some(("LOFT_NO_RANGE_ARITH", "1")),
        Some(("LOFT_NO_GUARDED_CHAIN", "1")),
        Some(("LOFT_NO_NN_FAST", "1")),
    ] {
        for falsifier in [
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_POISON_CLAIM", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
            ("LOFT_HOIST_VERIFY", "1"),
        ] {
            let mut env = vec![falsifier];
            env.extend(switch);
            let got = run(&["--native"], &env);
            assert_eq!(
                got, oracle,
                "native under {env:?} must print what the interpreter printed"
            );
        }
    }
}

#[test]
fn each_cell_emits_exactly_the_forms_predicted() {
    let rust = emit("on", &[]);
    let got = counts(&rust);
    for (name, expected) in EXPECTED {
        let row = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(row, *expected, "{name}: {KEYS:?}");
    }
}

#[test]
fn the_switch_restores_every_template() {
    let rust = emit("off", &[("LOFT_NO_RANGE_ARITH", "1")]);
    let got = counts(&rust);
    for (name, _) in EXPECTED {
        let row = got[*name];
        // No plain arithmetic anywhere.  a4 no longer needs its exemption: its loop has one
        // admitted operator, so the chain guard declines it and the `wrapping_mul` the
        // exemption existed for is gone.
        let plain = row[0] + row[1] + row[2] + row[3] + row[6] + row[7];
        assert_eq!(
            plain, 0,
            "{name}: LOFT_NO_RANGE_ARITH=1 leaves a plain operator: {row:?}"
        );
    }
    assert!(
        got["n_a1"][4] + got["n_a1"][5] >= 6,
        "a1's chain is back on its templates: {:?}",
        got["n_a1"]
    );
}
