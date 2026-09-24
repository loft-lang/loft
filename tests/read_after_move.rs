// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN163 P3 (`@FR-H-Spent`) — A NAME READ AFTER ITS VALUE MOVED IS A COMPILE-TIME ERROR.
//!
//! `(H-Move)` moves a value the function owns into the structure it is placed in, and `(H-Spent)`
//! makes the name spent from the end of that statement: reading it — or placing it a second time
//! — reads a value whose release the new structure now holds.  A move under a branch spends the
//! name on that path only, so a read any spending path reaches is refused, which is rustc's
//! reading of a possibly-moved value.  A reassignment refills the name.
//!
//! Every cell is a line of the matrix the check was built against, each with its answer written
//! before the run.  The legal half matters as much as the refused half: an error that fires on a
//! program the rules permit breaks that program, so each legal shape the walk has to tell apart
//! from a late read has its own cell — a read inside the moving statement, a refill, a move and a
//! refill in one arm, a move in an arm that returns, a type with no droppable, an argument, a
//! view, and a loop that moves and refills on every pass.
//!
//! On by default with the copy refusal, and switched off with it by `LOFT_NO_LEASE_REFUSE=1`.
//! Subprocess cells, as in `tests/lease_refuse.rs`: the switch is cached in a `OnceLock`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn check(source: &str, mode: &str, on: bool) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "loft_spent_{}_{}.loft",
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
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_LEASE_REFUSE");
    if !on {
        cmd.env("LOFT_NO_LEASE_REFUSE", "1");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

const PRELUDE: &str = "struct H { id: integer }\n\
                       struct Hold { h: H }\n\
                       struct N { h: H, tag: integer }\n\
                       struct P { x: integer }\n\
                       fn OpDrop(self: H) { print(\"D{self.id}\"); }\n\
                       fn mk(id: integer) -> H { return H { id: id }; }\n\
                       fn use_h(h: H) -> integer { h.id }\n";

/// The refused cells: `(name, body, the spent name the error must name)`.
const REFUSED: &[(&str, &str, &str)] = &[
    (
        "read after the move",
        "fn f() { a = mk(1); k = Hold { h: a }; print(\"{a.id} {k.h.id}\"); }",
        "a",
    ),
    (
        "read after a move in one arm",
        "fn f(c: boolean) { a = mk(2); if c { k = Hold { h: a }; print(\"{k.h.id}\"); } print(\"{a.id}\"); }",
        "a",
    ),
    (
        "a collection grown after it moved",
        "fn f() { v: vector<H> = [mk(3)]; d = v; v += [mk(4)]; print(\"{len(d)}\"); }",
        "v",
    ),
    (
        "placed twice",
        "fn f() { a = mk(5); k1 = Hold { h: a }; k2 = Hold { h: a }; print(\"{k1.h.id}{k2.h.id}\"); }",
        "a",
    ),
    (
        "returned after it moved",
        "fn f() -> H { a = mk(6); k = Hold { h: a }; print(\"{k.h.id}\"); return a; }",
        "a",
    ),
    (
        "read after a loop that moved it and broke out",
        "fn f(n: integer) { a = mk(7); for i in 0..n { if i == 1 { k = Hold { h: a }; print(\"{k.h.id}\"); break; } } print(\"{a.id}\"); }",
        "a",
    ),
    (
        "a tuple moved on every pass of a loop",
        "fn f() { t = (mk(8), 1); for i in 0..2 { u = t; print(\"{u.0.id} {i}\"); } }",
        "t",
    ),
];

/// The legal cells: each is a program the rules permit, and must compile without the error.
const LEGAL: &[(&str, &str)] = &[
    (
        "a read inside the moving statement",
        "fn f() { c = mk(10); k = N { h: c, tag: c.id }; print(\"{k.tag}\"); }",
    ),
    (
        "a refill before the read",
        "fn f() { a = mk(11); k = Hold { h: a }; a = mk(12); print(\"{a.id} {k.h.id}\"); }",
    ),
    (
        "a move and a refill in one arm",
        "fn f(c: boolean) { a = mk(13); if c { k = Hold { h: a }; print(\"{k.h.id}\"); a = mk(14); } print(\"{a.id}\"); }",
    ),
    (
        "a move in an arm that returns",
        "fn f(c: boolean) -> Hold { a = mk(15); if c { return Hold { h: a }; } print(\"{a.id}\"); Hold { h: mk(16) } }",
    ),
    (
        "a type with no droppable",
        "fn f() { p = P { x: 1 }; q = p; print(\"{p.x} {q.x}\"); }",
    ),
    (
        "an argument",
        "fn f() { a = mk(17); n = use_h(a); print(\"{n} {a.id}\"); }",
    ),
    (
        "a view of a member",
        "fn f() { k = Hold { h: mk(18) }; x = k.h; print(\"{x.id} {k.h.id}\"); }",
    ),
    (
        "a loop that moves and refills on every pass",
        "fn f() { for i in 0..2 { a = mk(20 + i); k = Hold { h: a }; print(\"{k.h.id}\"); } }",
    ),
];

fn with_main(body: &str) -> String {
    format!("{PRELUDE}{body}\nfn main() {{ }}\n")
}

#[test]
fn a_read_of_a_moved_name_is_refused_on_both_backends() {
    for (name, body, var) in REFUSED {
        for mode in ["--interpret", "--native"] {
            let out = check(&with_main(body), mode, true);
            assert!(
                out.contains("[read-after-move]")
                    && out.contains(&format!("`{var}` is read after")),
                "{name} ({mode}): expected read-after-move naming `{var}`, got:\n{out}"
            );
        }
    }
}

#[test]
fn a_legal_program_is_not_refused() {
    for (name, body) in LEGAL {
        let out = check(&with_main(body), "--interpret", true);
        assert!(
            !out.contains("read-after-move"),
            "{name}: a program the rules permit was refused:\n{out}"
        );
    }
}

/// The switch is part of the contract (the drop gate runs under it): with it off, the same
/// program compiles.  Without this the refused cells would pass just as well if the error were
/// raised unconditionally.
#[test]
fn with_the_switch_off_a_read_after_a_move_still_compiles() {
    let (_, body, _) = REFUSED[0];
    let out = check(&with_main(body), "--interpret", false);
    assert!(
        !out.contains("read-after-move"),
        "the switch is off:\n{out}"
    );
}
