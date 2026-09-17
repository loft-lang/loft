// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN163 P3 (`@FR-H-Copy-Refuse`) — A WRITTEN COPY OF A DROPPABLE IS A COMPILE-TIME ERROR.
//!
//! `(H-Copy-Refuse)` makes a copy of a type that owns a droppable without `OpCopy` an error on
//! the line that writes it: two structures on one resource release it twice, and the verdict is
//! read off that line and the declared types alone, so no later line can change it.  The census
//! (`LOFT_DROP_COPY_CENSUS`) has produced exactly these verdicts as a REPORT since P2r; this
//! raises them.
//!
//! Opt-in behind `LOFT_LEASE_REFUSE` while this repository's own corpus is converted — the rules
//! refuse 227 lines across 29 of the 38 files that declare `OpDrop`, and each is a guard pinning
//! the release machinery the refusal replaces.  That makes the SWITCH part of the contract, so
//! [`with_the_switch_off_the_copy_still_compiles`] pins it: a guard that only shows the error
//! would pass just as well if the gate had been wired shut, which is the half that has to stay
//! true until the flip.
//!
//! These are subprocess cells rather than `code!` ones deliberately.  `code!` runs in-process and
//! `keys.rs` caches every switch in a `OnceLock`, so the first test in a binary to read one fixes
//! it for all the others — an in-process cell could not have an on and an off case at once.
//! `tests/post_scope_lints_under_tests.rs` is the same shape for the same reason.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Compile `source` in `mode` and return `(stdout+stderr, exit code)`.
///
/// `--check` compiles without running: the refusal is raised in the post-scope lints, which both
/// the program path and the test path reach, and nothing here depends on the program's output.
fn check(tag: &str, source: &str, mode: &str, env: &[(&str, &str)]) -> (String, Option<i32>) {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "loft_lease_{tag}_{}_{}.loft",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).expect("write probe");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--check")
        .arg(mode)
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_ERRORS", "compact")
        .env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code(),
    )
}

const ON: &[(&str, &str)] = &[("LOFT_LEASE_REFUSE", "1")];

const PRELUDE: &str = "struct H { id: integer }\n\
                       fn OpDrop(self: H) { print(\"D{self.id}\"); }\n\
                       fn mk(id: integer) -> H { return H { id: id }; }\n\
                       struct S { h: H }\n\
                       fn take(x: H) { print(\"T{x.id}\"); }\n\
                       fn tup(t: (H, integer)) { print(\"U{t.0.id}\"); }\n";

fn program(body: &str) -> String {
    format!("{PRELUDE}{body}")
}

/// The plainest copy there is: `(B-Copy)` binds a whole value, and `H` owns a resource.
#[test]
fn a_whole_value_bind_of_a_droppable_is_refused() {
    let (out, code) = check(
        "bind",
        &program("fn main() { a = mk(1); b = a; print(\"{b.id}\"); }"),
        "--interpret",
        ON,
    );
    assert!(
        out.contains("[copy-of-droppable]"),
        "a whole-value bind of a droppable is the copy `(H-Copy-Refuse)` names first\n{out}"
    );
    assert!(
        out.contains("cannot copy `a` here"),
        "the error names the copy — the value the author wrote, not the destination\n{out}"
    );
    assert!(
        out.contains("pass it as an argument"),
        "and what to write instead: the rule requires the rewrite, not just the refusal\n{out}"
    );
    assert_ne!(code, Some(0), "a refused program must not compile\n{out}");
}

/// The SWITCH is part of the contract until the corpus is converted, so it is pinned from both
/// sides.  Without this, a guard asserting only the error would pass with the gate wired shut.
#[test]
fn with_the_switch_off_the_copy_still_compiles() {
    let (out, code) = check(
        "off",
        &program("fn main() { a = mk(1); b = a; print(\"{b.id}\"); }"),
        "--interpret",
        &[],
    );
    assert!(
        !out.contains("copy-of-droppable"),
        "the refusal is opt-in while the corpus is converted\n{out}"
    );
    assert_eq!(code, Some(0), "and the program still compiles\n{out}");
}

/// The shapes `(H-Copy-Refuse)` calls LEGAL, each measured legal before the refusal was built.
///
/// This is the over-reach half: the refusal's population is "an existing value placed in a new
/// structure", and every cell here places none.  An argument binds without copying
/// (`calls.md` F-ParamHeap), a `return` of the function's own variable is `(H-Move)`, and a view
/// of a member is `(B-View)` — including an ELEMENT read, which is the cell that corrected a
/// prediction of mine: `v[0]` names a place inside `v`, so binding it copies nothing.
#[test]
fn the_shapes_the_rule_calls_legal_stay_legal() {
    for (name, body) in [
        ("argument", "fn main() { a = mk(1); take(a); }"),
        ("fresh_argument", "fn main() { take(mk(1)); }"),
        ("tuple_argument", "fn main() { t = (mk(1), 5); tup(t); }"),
        (
            "return_own_local",
            "fn f() -> S { s = S { h: mk(1) }; return s; }\n\
             fn main() { r = f(); print(\"{r.h.id}\"); }",
        ),
        (
            "member_view",
            "fn main() { b = S { h: mk(1) }; x = b.h; print(\"{x.id}\"); }",
        ),
        (
            "element_read",
            "fn main() { v: vector<H> = [mk(1)]; x = v[0]; print(\"{x.id}\"); }",
        ),
        (
            "fresh_append",
            "fn main() { v: vector<H> = []; v += [mk(1)]; print(\"{len(v)}\"); }",
        ),
    ] {
        let (out, code) = check(name, &program(body), "--interpret", ON);
        assert!(
            !out.contains("copy-of-droppable"),
            "`{name}` makes no second structure, so the rule permits it — refusing it would \
             reject a sound program\n{out}"
        );
        assert_eq!(code, Some(0), "`{name}` must still compile\n{out}");
    }
}

/// The verdict is read off the IR after the scope pass, before either backend generates, so the
/// two cannot disagree — pinned rather than assumed, because a refusal that fired on one backend
/// only would make a program's legality depend on how it is run.
#[test]
fn both_backends_refuse_the_same_lines() {
    let src = program("fn main() { a = mk(1); b = a; t = (a, 5); print(\"{b.id}{t.0.id}\"); }");
    let lines = |out: &str| -> Vec<String> {
        let mut v: Vec<String> = out
            .lines()
            .filter(|l| l.contains("[copy-of-droppable]"))
            .filter_map(|l| l.rsplit_once(".loft:").map(|(_, p)| p.to_string()))
            .collect();
        v.sort();
        v
    };
    let (interp, _) = check("bk_i", &src, "--interpret", ON);
    let (native, _) = check("bk_n", &src, "--native", ON);
    assert!(
        !lines(&interp).is_empty(),
        "the cell must refuse at all, or this compares two empty lists\n{interp}"
    );
    assert_eq!(
        lines(&interp),
        lines(&native),
        "the refusal is raised before generation, so both backends name the same \
         lines\n--interpret:\n{interp}\n--native:\n{native}"
    );
}

/// A refusal's variable is not always one the author can see: a lifted join arm or a call-result
/// projection leaves it a `__lift_N`, measured on the corpus as `refuse:container:__lift_2`.
/// `Frame::author_name` walks the view temps back to what they stand for, and where nothing the
/// author wrote is reachable the sentence describes the value by its TYPE instead.
///
/// Naming the temp would describe the compiler's workings to someone reading about their own
/// program — the shape loft#1453 already paid for once.
#[test]
fn no_message_names_a_compiler_temp() {
    let (out, _) = check(
        "temp",
        &program(
            "fn make() -> S { return S { h: mk(1) }; }\n\
             fn main() { x = make().h; v: vector<H> = []; v += [x]; print(\"{len(v)}\"); }",
        ),
        "--interpret",
        ON,
    );
    assert!(
        out.contains("[copy-of-droppable]"),
        "a member of a call result placed in a container is a written copy\n{out}"
    );
    for temp in ["__lift", "__ref", "__vdb", "__disp", "_elm_", "_tuphold"] {
        assert!(
            !out.contains(temp),
            "no message may name the compiler's own variable `{temp}` — the reader did not \
             write it and cannot act on it\n{out}"
        );
    }
}
