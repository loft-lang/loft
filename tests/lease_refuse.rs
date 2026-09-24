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
//! On by default; `LOFT_NO_LEASE_REFUSE=1` switches it off, which is what the drop gate runs its
//! cells under (a refused cell measures no release).  That makes the SWITCH part of the
//! contract, so [`with_the_switch_off_the_copy_still_compiles`] pins it from the other side: a
//! guard that only shows the error would pass just as well if the switch were wired shut.
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

/// The refusal is the default; the cells name no environment for it.
const ON: &[(&str, &str)] = &[];

const PRELUDE: &str = "struct H { id: integer }\n\
                       fn OpDrop(self: H) { print(\"D{self.id}\"); }\n\
                       fn mk(id: integer) -> H { return H { id: id }; }\n\
                       struct S { h: H }\n\
                       fn take(x: H) { print(\"T{x.id}\"); }\n\
                       fn tup(t: (H, integer)) { print(\"U{t.0.id}\"); }\n";

fn program(body: &str) -> String {
    format!("{PRELUDE}{body}")
}

/// The plainest copy LEFT after the 2026-09-17 ruling: a value the function does not own.
///
/// ⚠ This cell used to be `a = mk(1); b = a`, and that is no longer a copy at all — `(H-Move)`
/// moves a value the function OWNS.  The refusal's population is now what the function does NOT
/// own, and a parameter is the clearest member of it: the caller still holds what it passed.
#[test]
fn a_copy_of_what_the_caller_owns_is_refused() {
    let (out, code) = check(
        "param",
        &program(
            "fn wrap(c: H) -> S { return S { h: c }; }\n\
             fn main() { s = wrap(mk(1)); print(\"{s.h.id}\"); }",
        ),
        "--interpret",
        ON,
    );
    assert!(
        out.contains("[copy-of-droppable]"),
        "placing what the caller owns is the copy `(H-Copy-Refuse)` still names\n{out}"
    );
    assert!(
        out.contains("cannot copy `c` here"),
        "the error names the copy — the value the author wrote, not the destination\n{out}"
    );
    assert!(
        out.contains("the caller still owns what it passed"),
        "and WHY, in this population's own words: the reason is the caller's ownership, not a \
         second structure in general\n{out}"
    );
    assert!(
        out.contains("return the owner"),
        "and what to write instead: the rule requires the rewrite, not just the refusal\n{out}"
    );
    assert_ne!(code, Some(0), "a refused program must not compile\n{out}");
}

/// A copy the compiler later SKIPS is judged by the line that wrote it.  `(H-Copy-Refuse)` reads a
/// copy off its own line "whatever the program does after that line", and `(H-Elide)` may elide a
/// copy only after that verdict.  Two copies had no IR left by the time the census looked:
///
/// - `u = p` of a vector parameter that nothing mutates, which the borrow elision replaces by
///   reads of `p` — while the same bind followed by a growth kept its copy and was refused, so a
///   LATER line decided validity;
/// - `p = p`, which the parser erases (#330), and which was judged only when the census's
///   environment variable happened to be set.
///
/// The controls are the same two spellings on a value the function OWNS, which move.
#[test]
fn a_copy_the_compiler_skips_is_judged_by_its_own_line() {
    for (tag, body) in [
        (
            "elided_vec",
            "fn keep(p: vector<H>) { u = p; print(\"R{len(u)}\"); }\n\
             fn main() { v: vector<H> = [mk(1)]; keep(v); }",
        ),
        (
            "grown_vec",
            "fn keep(p: vector<H>) { u = p; u += [mk(2)]; print(\"R{len(u)}\"); }\n\
             fn main() { v: vector<H> = [mk(1)]; keep(v); }",
        ),
        (
            "self_param",
            "fn keep(p: H) { p = p; print(\"R{p.id}\"); }\n\
             fn main() { a = mk(1); keep(a); }",
        ),
    ] {
        for mode in ["--interpret", "--native"] {
            let (out, code) = check(tag, &program(body), mode, ON);
            assert!(
                out.contains("[copy-of-droppable]") && out.contains("cannot copy `p` here"),
                "{tag} {mode}: the copy of the parameter is refused on its own line\n{out}"
            );
            assert_ne!(
                code,
                Some(0),
                "{tag} {mode}: a refused program must not compile\n{out}"
            );
        }
    }
    for (tag, body) in [
        (
            "owned_vec",
            "fn vs() -> vector<H> { [mk(1), mk(2)] }\n\
             fn main() { w = vs(); u = w; print(\"R{len(u)}\"); }",
        ),
        (
            "owned_self",
            "fn main() { a = mk(1); a = a; print(\"R{a.id}\"); }",
        ),
    ] {
        let (out, code) = check(tag, &program(body), "--interpret", ON);
        assert!(
            !out.contains("[copy-of-droppable]") && code == Some(0),
            "{tag}: a value the function owns moves, and stays legal\n{out}"
        );
    }
}

/// A value the function OWNS is a MOVE, wherever it is placed — the ruling's whole point, and
/// the half that would go unnoticed if only the refusals were pinned.
///
/// Each of these was REFUSED before 2026-09-17 and had no legal spelling: a function could not
/// accept a resource and store it, and naming one in order to validate it forfeited placing it.
#[test]
fn a_value_the_function_owns_moves_wherever_it_is_placed() {
    for (name, body) in [
        (
            "name_then_wrap",
            "fn lower(id: integer) -> H { return mk(id); }\n\
             fn upper(id: integer) -> S { c = lower(id); return S { h: c }; }\n\
             fn main() { print(\"{upper(1).h.id}\"); }",
        ),
        (
            "validate_then_store",
            "fn f(id: integer) -> S { c = mk(id); if c.id > 0 { return S { h: c }; } \
             return S { h: mk(0) }; }\n\
             fn main() { print(\"{f(1).h.id}\"); }",
        ),
        (
            "open_several_then_collect",
            "fn main() { a = mk(1); b = mk(2); v: vector<H> = [a, b]; print(\"{len(v)}\"); }",
        ),
        (
            "use_then_store",
            "fn main() { c = mk(1); n = c.id; v: vector<H> = [c]; print(\"{n}{len(v)}\"); }",
        ),
        // The shapes a first version of the refusal wrongly refused (found converting the
        // corpus, 2026-09-23), each a case the rules name as legal.  A tuple local RETURNED —
        // as the tail, and by `return`, with a nullable member copied through a stash:
        (
            "tuple_returned_as_tail",
            "fn f() -> (integer, H) { a = (1, mk(1)); a }\n\
             fn main() { t = f(); print(\"{t.1.id}\"); }",
        ),
        (
            "tuple_returned_nullable_member",
            "fn g(id: integer) -> H? { if id == 0 { return null; } mk(id) }\n\
             fn f(id: integer) -> (integer, H?) { a = (1, g(id)); return a; }\n\
             fn main() { t = f(5); print(\"{t.1?.id ?? 0}\"); }",
        ),
        // A whole-tuple bind of a tuple the function owns, whose member is a vector:
        (
            "tuple_with_vector_member_moved",
            "fn main() { t = ([mk(2)], 1); u = t; print(\"{len(u.0)}\"); }",
        ),
        // A vector literal inside a tuple literal: the literal's backing is its own storage.
        (
            "vector_literal_in_tuple",
            "fn main() { t = ([mk(3)], 1); print(\"{len(t.0)}\"); }",
        ),
        // A member of a call result handed to a function as an argument:
        (
            "call_member_as_argument",
            "fn box() -> S { S { h: mk(4) } }\n\
             fn main() { take(box().h); }",
        ),
    ] {
        let (out, code) = check(name, &program(body), "--interpret", ON);
        assert!(
            !out.contains("copy-of-droppable"),
            "`{name}` places a value the function owns, which `(H-Move)` moves — refusing it \
             would reject the construction the ruling exists to allow\n{out}"
        );
        assert_eq!(code, Some(0), "`{name}` must compile\n{out}");
    }
}

/// The SWITCH is part of the contract — the drop gate runs under it — so it is pinned from both
/// sides.  Without this, a guard asserting only the error would pass with the switch wired shut.
#[test]
fn with_the_switch_off_the_copy_still_compiles() {
    let (out, code) = check(
        "off",
        &program(
            "fn wrap(c: H) -> S { return S { h: c }; }\n\
             fn main() { s = wrap(mk(1)); print(\"{s.h.id}\"); }",
        ),
        "--interpret",
        &[("LOFT_NO_LEASE_REFUSE", "1")],
    );
    assert!(
        !out.contains("copy-of-droppable"),
        "`LOFT_NO_LEASE_REFUSE=1` switches the refusal off\n{out}"
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
    let src = program(
        "fn wrap(c: H) -> S { return S { h: c }; }\n\
         fn rewrap(s: S) -> S { return S { h: s.h }; }\n\
         fn main() { a = wrap(mk(1)); b = rewrap(a); print(\"{b.h.id}\"); }",
    );
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
