// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-H-Sound` — a program the checker accepts never reads a store after its free, on either
//! backend.
//!
//! An ordinary run cannot see such a read: a freed arena slot keeps its bytes, so a stale read
//! answers the right value until the slot is handed to someone else.  Each cell here therefore
//! runs with both instruments armed.  `LOFT_POISON` overwrites a freed store, and
//! `LOFT_STRICT_STORES` never recycles a slot, so every read of a freed store is reported by name.
//! A cell passes when it prints its hand-computed value, exits cleanly and reports no
//! use-after-free.  These are subprocess cells because both switches are read once per process.
//!
//! The nightly `LOFT_POISON` sweep runs only the interpreter, and every defect below was
//! `--native`-only:
//! - a call handed a POOLED buffer (`@FR-O-Buffer`) adopts it on one pass and mints on the next,
//!   and the rebind freed the displaced store, which was the buffer itself.  The interpreter
//!   already asked whether the displaced store was the buffer; both backends now ask
//!   `Function::displaced_buffer_witness`.
//! - a loop buffer (`@FR-R-LoopBuffer`) was kept across iterations while a local bound from its
//!   field owned the store and freed it at the iteration's end.  `hoist::loop_buffers` now
//!   declines such a buffer.
//!
//! The controls are the shapes one step away, which were right.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

const RB: &str = "struct Rb { name: text, n: integer }\n\
                  fn rb_mk(k: integer) -> Rb { Rb { name: \"r{k}\", n: k } }\n\
                  fn rb_cond(c: integer) -> Rb { \
                  if c > 0 { r = rb_mk(1); if r.n > 99 { return r; } } rb_mk(3) }\n";

/// Run `body` in `mode` with both instruments armed; answer the `R` line, or what went wrong.
fn run(tag: &str, body: &str, mode: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "loft_heap_sound_{tag}_{}_{}.loft",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, format!("{body}\n")).expect("write cell");
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg(mode)
        .arg(&path)
        .env("LOFT_TIMEOUT", "240")
        .env("LOFT_POISON", "1")
        .env("LOFT_STRICT_STORES", "1")
        .output()
        .expect("run loft");
    let _ = std::fs::remove_file(&path);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let uaf = text.matches("USE AFTER FREE").count() + text.matches("out of bounds").count();
    let value = text.lines().find(|l| l.starts_with('R')).unwrap_or("");
    format!("{value} exit={} uaf={uaf}", out.status.code().unwrap_or(-1))
}

fn check(cells: &[(&str, String, &str)]) {
    let mut wrong = Vec::new();
    for (tag, body, want) in cells {
        let want = format!("{want} exit=0 uaf=0");
        for mode in ["--interpret", "--native"] {
            let got = run(tag, body, mode);
            if got != want {
                wrong.push(format!("{tag} {mode}: got `{got}`, want `{want}`"));
            }
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

/// A call handed a pooled buffer, in a loop whose passes alternate between adopting the buffer
/// and minting a store of the callee's own.  The rebind must not free the buffer it displaces.
#[test]
fn a_rebind_never_frees_the_pooled_buffer_its_call_was_handed() {
    check(&[
        (
            "alternating",
            format!(
                "{RB}fn main() {{ t = 0; for k in 0..6 {{ t += rb_cond(k % 2).n; }} println(\"R{{t}}\"); }}"
            ),
            "R18",
        ),
        // The same rebind in a generator, whose buffer is a coroutine field.
        (
            "generator",
            format!(
                "{RB}fn gen() -> iterator<integer> {{ for k in 0..6 {{ x = rb_cond(k % 2); yield x.n; }} }}\n\
                 fn main() {{ t = 0; for v in gen() {{ t += v; }} println(\"R{{t}}\"); }}"
            ),
            "R18",
        ),
        // CONTROL: a local declared before the loop and rebound from the same call.
        (
            "declared_outside",
            format!(
                "{RB}fn main() {{ t = 0; x = rb_cond(0); \
                 for k in 0..6 {{ x = rb_cond(k % 2); t += x.n; }} println(\"R{{t}} {{x.name}}\"); }}"
            ),
            "R18 r3",
        ),
        // CONTROL: a vector result, whose buffer the callee does not adopt this way.
        (
            "vector_result",
            "fn vc(c: integer) -> vector<integer> { if c > 0 { r = [1, 2]; \
             if len(r) > 9 { return r; } } [3, 4] }\n\
             fn main() { t = 0; for k in 0..6 { t += vc(k % 2)[1]; } println(\"R{t}\"); }"
                .to_string(),
            "R24",
        ),
    ]);
}

/// A loop buffer whose field is bound to a local that owns the store: a value branch whose other
/// arm mints, in one loop, in nested loops and at two assignment sites.  The buffer must not be
/// kept across iterations, since the local frees its store at the iteration's end.
#[test]
fn a_loop_buffer_is_not_kept_when_a_local_frees_its_store() {
    const CP: &str = "cp = [41, 42, 43]; m = fn(k: integer) -> vector<integer> { [7, 8] }; s = 0;";
    check(&[
        (
            "closure_arm",
            format!(
                "fn main() {{ {CP} for i in 0..6 {{ r = if i % 2 == 0 {{ m(0) }} else {{ cp }}; \
                 s += r[1]; }} println(\"R{{s}} {{cp[1]}}\"); }}"
            ),
            "R150 42",
        ),
        (
            "nested_loops",
            format!(
                "fn main() {{ {CP} for j in 0..2 {{ for i in 0..3 {{ \
                 r = if i % 2 == 0 {{ m(0) }} else {{ cp }}; s += r[1]; }} }} \
                 println(\"R{{s}} {{cp[1]}}\"); }}"
            ),
            "R116 42",
        ),
        (
            "two_sites",
            format!(
                "fn main() {{ {CP} for i in 0..6 {{ r: vector<integer> = []; \
                 if i % 2 == 0 {{ r = m(0); }} else {{ r = cp; }} s += r[1]; }} \
                 println(\"R{{s}} {{cp[1]}}\"); }}"
            ),
            "R150 42",
        ),
        // CONTROLS: a named function's mint and a literal arm, beside the same variable arm.
        (
            "named_fn_arm",
            "fn mk(k: integer) -> vector<integer> { [k, k + 1] }\n\
             fn main() { cp = [41, 42, 43]; s = 0; \
             for i in 0..6 { r = if i % 2 == 0 { mk(7) } else { cp }; s += r[1]; } \
             println(\"R{s} {cp[1]}\"); }"
                .to_string(),
            "R150 42",
        ),
        (
            "literal_arm",
            "fn main() { cp = [41, 42, 43]; s = 0; \
             for i in 0..6 { r = if i % 2 == 0 { [7, 8] } else { cp }; s += r[1]; } \
             println(\"R{s} {cp[1]}\"); }"
                .to_string(),
            "R150 42",
        ),
    ]);
}
