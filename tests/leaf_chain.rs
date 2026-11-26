// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN203 rust-writer: a frameless helper is the Rust writer's leaf inlining — its `--native` runs as `--native-rust`.
//! Chains of helper calls on the compiled tiers: the cells
//! (`plans/157-native-4x-drawing/bytecode-comparisons/N4b-leaf-chain-cells.loft`) carry one
//! call-tree shape per case.  These tests pin that the lean build (`--ship`) answers what the
//! interpreter answers without a leak, and that runaway recursion — through a helper that calls
//! no further, an overload member, a generic instance or a method — ends in the depth cap's
//! report on the development build (`--native`) rather than a native stack crash.  A shipped
//! build has no cap: it recurses as deep as the hardware allows, then crashes, and `loft` names
//! the crash (`ship_answers_or_names_its_crash`).
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/N4b-leaf-chain-cells.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    // The cells pin which FRAMES survive; `@FR-R-InlineLeaf` would replace the scalar leaves
    // (`lower_byte`) by their bodies, and then there is no function left to read a frame of.
    cmd.args(args)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_INLINE_LEAF", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

/// A `--ship` run of a deep recursion: no depth cap, so it either answers — a C compiler may
/// turn `f(n - 1) + 1` into a loop — or runs out of stack and crashes, which `loft` names with
/// a non-zero exit.  Never the development cap's report, and never a silent exit.
fn ship_answers_or_names_its_crash(file: &str, res: &Output, answer: &str) {
    let out = String::from_utf8_lossy(&res.stdout);
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(
        !err.contains("call stack overflow"),
        "{file} --ship: a shipped build has no depth cap:\n{err}"
    );
    if res.status.success() {
        assert_eq!(out.trim(), answer, "{file} --ship answered wrong:\n{err}");
    } else {
        assert!(
            err.contains("error: the program crashed: "),
            "{file} --ship: a crash is named, exit {:?}:\n{err}",
            res.status.code()
        );
    }
}

fn cells() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CELLS)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn the_lean_build_answers_what_the_interpreter_answers() {
    let src = cells();
    let interp = loft(&["--interpret", &src], &[]);
    let lean = loft(
        &["--ship", &src],
        &[("LOFT_NATIVE_LEAK_CHECK", "1"), ("LOFT_STRICT_STORES", "1")],
    );
    let expected = "c1 97 99 -1 -1\nc2 3\nc3 3628800\nc4 true true false\nc5 240\nc6 41\n\
                    c7 5\nc8 1194\nc9 3 104 121\nc9 3 104 121\nc9 3 104 121\nc10 8\n";
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout),
        expected,
        "the oracle moved"
    );
    assert_eq!(
        String::from_utf8_lossy(&lean.stdout),
        expected,
        "lean stderr: {}",
        String::from_utf8_lossy(&lean.stderr)
    );
    assert!(lean.status.success(), "lean exit {:?}", lean.status);
    let err = String::from_utf8_lossy(&lean.stderr);
    assert!(
        !err.contains("not freed") && !err.contains("strict-store"),
        "the lean build leaked or read a freed store:\n{err}"
    );
}

/// The development build's cap holds through a helper that calls no further.  Only `--native`:
/// a shipped build has no cap, and this recursion never ends, so a C compiler that turns it
/// into a loop would spin until the timeout (`tests/native.rs` pins a shipped crash).
#[test]
fn runaway_recursion_through_a_frameless_helper_hits_the_depth_cap() {
    let dir = std::env::temp_dir().join(format!("loft_leaf_chain_runaway_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let prog = dir.join("runaway.loft");
    fa::write(
        &prog,
        "fn at(s: text, i: integer) -> integer { if i < 0 || i >= size(s) { -1 } else { s.byte_at(i) } }\n\
         fn pick(n: integer) -> integer { at(\"ab\", n % 2) }\n\
         fn runaway(n: integer) -> integer { pick(n) + runaway(n + 1) }\n\
         fn main() { println(\"{runaway(0)}\"); }\n",
    )
    .expect("write the probe");
    let res = loft(&["--native", &prog.to_string_lossy()], &[]);
    let err = String::from_utf8_lossy(&res.stderr);
    assert_eq!(
        res.status.code(),
        Some(1),
        "a clean fault exit, not a signal:\n{err}"
    );
    assert!(
        err.contains("call stack overflow"),
        "the depth cap must report the runaway recursion:\n{err}"
    );
    let _ = fa::remove_dir_all(&dir);
}

/// An overload member (`f_…`) and an instance of a free generic (`i_…_n_…`) are free functions
/// in all but their key, so a runaway recursion through either stops at loft's depth cap on
/// `--native` as its `n_` twin does — it overflowed Rust's own stack (the member) or ran past
/// the cap unchecked (the instance), because the prelude that enforces the cap was written for
/// `n_` names alone.  `--ship` has no cap: it answers or names its crash.
#[test]
fn runaway_recursion_through_an_overload_member_or_a_generic_hits_the_depth_cap() {
    let dir = std::env::temp_dir().join(format!("loft_leaf_chain_keyed_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let cases = [
        (
            "member.loft",
            "struct P { x: integer }\nstruct Q { y: integer }\n\
             fn depth(a: P, n: integer) -> integer { if n == 0 { 0 } else { depth(a, n - 1) + 1 } }\n\
             fn depth(a: Q, n: integer) -> integer { n }\n\
             fn main() { println(\"{depth(P { x: 1 }, 10000000)}\"); }\n",
        ),
        (
            "instance.loft",
            "fn depth<T>(a: T, n: integer) -> integer { if n == 0 { 0 } else { depth(a, n - 1) + 1 } }\n\
             fn main() { println(\"{depth(1, 10000000)}\"); }\n",
        ),
    ];
    for (file, src) in cases {
        let prog = dir.join(file);
        fa::write(&prog, src).expect("write the probe");
        ship_answers_or_names_its_crash(
            file,
            &loft(&["--ship", &prog.to_string_lossy()], &[]),
            "10000000",
        );
        let res = loft(&["--native", &prog.to_string_lossy()], &[]);
        let err = String::from_utf8_lossy(&res.stderr);
        assert_eq!(
            res.status.code(),
            Some(1),
            "{file} --native: a clean fault exit, not a signal:\n{err}"
        );
        assert!(
            err.contains("call stack overflow"),
            "{file} --native: the depth cap must report the runaway recursion:\n{err}"
        );
    }
    let _ = fa::remove_dir_all(&dir);
}

/// A loft-bodied METHOD is a call frame like a free function: a runaway recursion through one
/// stops at loft's depth cap on `--native` as it does on the interpreter, rather than
/// overflowing the machine's stack — alone, mutually with another method, and through a method
/// on a generic receiver — and `stack_trace()` names its frames as the interpreter names them.
/// `--ship` has no cap: it answers or names its crash.
#[test]
fn runaway_recursion_through_a_method_hits_the_depth_cap() {
    let dir = std::env::temp_dir().join(format!("loft_leaf_chain_method_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let cases = [
        (
            "self.loft",
            "struct P { x: integer }\n\
             fn depth(self: P, n: integer) -> integer { if n == 0 { 0 } else { self.depth(n - 1) + 1 } }\n\
             fn main() { p = P { x: 1 }; println(\"{p.depth(10000000)}\"); }\n",
        ),
        (
            "mutual.loft",
            "struct P { x: integer }\nstruct Q { y: integer }\n\
             fn ping(self: P, q: Q, n: integer) -> integer { if n == 0 { 0 } else { q.pong(self, n - 1) + 1 } }\n\
             fn pong(self: Q, p: P, n: integer) -> integer { if n == 0 { 0 } else { p.ping(self, n - 1) + 1 } }\n\
             fn main() { println(\"{P { x: 1 }.ping(Q { y: 2 }, 10000000)}\"); }\n",
        ),
        (
            "generic.loft",
            "fn depth<T>(self: vector<T>, n: integer) -> integer { if n == 0 { 0 } else { self.depth(n - 1) + 1 } }\n\
             fn main() { v: vector<integer> = [1]; println(\"{v.depth(10000000)}\"); }\n",
        ),
    ];
    for (file, src) in cases {
        let prog = dir.join(file);
        fa::write(&prog, src).expect("write the probe");
        ship_answers_or_names_its_crash(
            file,
            &loft(&["--ship", &prog.to_string_lossy()], &[]),
            "10000000",
        );
        let res = loft(&["--native", &prog.to_string_lossy()], &[]);
        let err = String::from_utf8_lossy(&res.stderr);
        assert_eq!(
            res.status.code(),
            Some(1),
            "{file} --native: a clean fault exit, not a signal:\n{err}"
        );
        assert!(
            err.contains("call stack overflow"),
            "{file} --native: the depth cap must report the runaway recursion:\n{err}"
        );
    }
    let prog = dir.join("frames.loft");
    fa::write(
        &prog,
        "struct P { x: integer }\n\
         fn show(self: P, n: integer) -> integer {\n\
           if n == 0 { for f in stack_trace() { println(\"frame {f.function}\"); } self.x }\n\
           else { self.show(n - 1) + 1 }\n\
         }\n\
         fn main() { println(\"{P { x: 7 }.show(2)}\"); }\n",
    )
    .expect("write the probe");
    let expected = "frame main\nframe t_1P_show\nframe t_1P_show\nframe t_1P_show\n9\n";
    for mode in ["--interpret", "--native"] {
        let res = loft(&[mode, &prog.to_string_lossy()], &[]);
        assert_eq!(
            String::from_utf8_lossy(&res.stdout),
            expected,
            "{mode}: {}",
            String::from_utf8_lossy(&res.stderr)
        );
    }
    let _ = fa::remove_dir_all(&dir);
}
