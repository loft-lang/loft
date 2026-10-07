// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-InlineLeaf`, `@FR-R-MaskRange`, `@FR-R-SingleUse`, `@FR-R-ScaleFold` — PINNED on the
//! cells of `tests/scripts/a-leaf-call-inlined-answers-what-the-call-answers.loft`.  Those cells
//! check values, which pass whether or not a rule fires; this pins WHERE each fires, read off
//! `LOFT_TRACE_INLINE_LEAF`, so a rule that stops firing — or starts firing where a cell says it
//! must not — is a red test rather than a quiet slowdown or a lucky value.  It also runs the
//! cells under every switch, since the default run never reaches the paths a switch restores,
//! and checks the drawing hash loop the rules were built for no longer calls its leaf.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-leaf-call-inlined-answers-what-the-call-answers.loft";

/// `(test function, calls inlined, masks and fallbacks dropped, single uses, scale folds)`.
const FIRES: &[(&str, usize, usize, usize, usize)] = &[
    // Literal seed and salt: the second mask and the fallback go in each of the three calls
    // (the loop's, the assert's and its message's); the first mask stays, `i * 19349663`
    // exceeds it.
    ("n_test_the_hash_by_literal_arguments", 3, 6, 3, 3),
    // `s = 3`, `t = 9` are ranged locals, so even the first mask's operand fits.
    ("n_test_the_hash_by_variable_arguments", 2, 6, 2, 2),
    // A null seed: nothing is proven, everything stays.
    ("n_test_the_hash_of_a_null_seed_is_its_fallback", 2, 0, 0, 0),
    (
        "n_test_a_reassigned_parameter_is_never_substituted",
        2,
        0,
        2,
        0,
    ),
    // Only `k & 7` in `part(k)` goes; `& 6`, `masked`'s and `negxor`'s masks stay.
    ("n_test_masks_that_must_stay", 8, 2, 0, 0),
    ("n_test_a_value_read_twice_is_kept", 2, 0, 0, 0),
    // `halfnull(10)` loses its fallback and folds its scale; `halfnull(v[0])` keeps both.
    ("n_test_a_fallback_over_a_possible_null_stays", 4, 2, 0, 2),
    ("n_test_scales_that_do_not_fold", 6, 0, 0, 0),
    ("n_test_arguments_evaluate_in_order", 1, 0, 0, 0),
    // The tuple clause: `step`'s four calls; `named` holds text and stays a call.
    ("n_test_a_tuple_result", 4, 0, 0, 0),
];

const SWITCHES: [&str; 4] = [
    "LOFT_NO_INLINE_LEAF",
    "LOFT_NO_MASK_RANGE",
    "LOFT_NO_SINGLE_USE",
    "LOFT_NO_SCALE_FOLD",
];

fn loft(args: &[&str], file: &Path, env: &[(&str, &str)]) -> (String, String, bool) {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .arg(file)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_TRACE_INLINE_LEAF");
    for s in SWITCHES {
        cmd.env_remove(s);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_rule_fires_where_the_cells_say_and_nowhere_else() {
    let (_, err, ok) = loft(
        &["--interpret"],
        &cells(),
        &[("LOFT_TRACE_INLINE_LEAF", "1")],
    );
    assert!(ok, "the cells failed:\n{err}");
    for (f, calls, masks, singles, scales) in FIRES {
        let want = format!(
            "inline-leaf: fn {f} — {calls} call(s) inlined, {masks} mask/fallback(s), \
             {singles} single use(s), {scales} scale fold(s)"
        );
        assert!(
            err.lines().any(|l| l == want),
            "{f}: expected `{want}` in the trace:\n{err}"
        );
    }
    assert!(
        err.lines()
            .any(|l| l.starts_with("inline-leaf: n_tally declined:")),
        "a leaf with a reference parameter is declined:\n{err}"
    );
    assert!(
        err.lines()
            .any(|l| l.starts_with("inline-leaf: n_named declined: a non-scalar result")),
        "a tuple holding text is declined:\n{err}"
    );
}

#[test]
fn the_cells_hold_under_every_switch() {
    for s in SWITCHES {
        let (out, err, ok) = loft(&["--interpret"], &cells(), &[(s, "1")]);
        assert!(
            ok && out.trim_end().ends_with("ok"),
            "under {s}:\n{out}\n{err}"
        );
    }
}

#[test]
fn the_hash_loop_no_longer_calls_its_leaf() {
    let dir = std::env::temp_dir().join(format!("loft_leaf_inline_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join("hash.loft");
    fa::write(
        &file,
        "pub fn seed_hash(hseed: integer, hidx: integer, hsalt: integer) -> float {\n\
         \x20 hx = ((hseed * 73856093) ^ (hidx * 19349663) ^ (hsalt * 83492791)) & 0xFFFFFFFF;\n\
         \x20 hx = (hx ^ (hx >> 13)) & 0xFFFFFFFF;\n\
         \x20 hx = (hx * 1274126177) & 0xFFFFFFFF;\n\
         \x20 ((hx as float) / 4294967295.0 ?? 0.0) * 2.0 - 1.0\n}\n\
         fn main() {\n  acc = 0.0;\n  for i in 0..100000 { acc += seed_hash(1, i, 7); }\n\
         \x20 println(\"{(acc * 1000.0) as integer}\");\n}\n",
    )
    .expect("write program");
    let main_code = |dump: &str| -> String {
        let start = dump
            .match_indices("byte-code for ")
            .map(|(i, _)| i)
            .find(|&i| {
                dump[i..]
                    .lines()
                    .next()
                    .is_some_and(|l| l.contains(":n_main("))
            })
            .expect("main's bytecode");
        dump[start..].to_string()
    };
    let (on, _, _) = loft(&["introspect"], &file, &[]);
    let (off, _, _) = loft(&["introspect"], &file, &[("LOFT_NO_INLINE_LEAF", "1")]);
    assert!(
        !main_code(&on).contains("fn=n_seed_hash"),
        "the leaf is inlined into the loop"
    );
    assert!(
        main_code(&off).contains("fn=n_seed_hash"),
        "LOFT_NO_INLINE_LEAF keeps the call"
    );
    // The divisor the scale fold leaves (`4294967295 / 2`), and no nullable division.
    assert!(main_code(&on).contains("ConstFloat(val=2147483647.5)"));
    assert!(!main_code(&on).contains("DivFloatNullable"));
    let (a, _, ok_a) = loft(&["--interpret"], &file, &[]);
    let (b, _, ok_b) = loft(&["--interpret"], &file, &[("LOFT_NO_INLINE_LEAF", "1")]);
    assert!(ok_a && ok_b && a == b, "same answer either way: {a} / {b}");
    let _ = fa::remove_file(&file);
}
