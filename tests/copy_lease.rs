// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN163 P4 (`@FR-H-Copy-Lease`) — A COPY OF A TYPE THAT DECLARES `OpCopy` TAKES A LEASE.
//!
//! `(H-Copy-Lease)`: a copy copies the bytes and then runs `OpCopy` on the NEW structure, and a
//! struct holding such a member gets a synthesized cascade — its own `OpCopy` first, then its
//! members'.  Two structures, two leases, two drops.  A MOVE runs nothing, and neither does a
//! byte move such as a vector's growth.
//!
//! Every trace below is computed by hand from the rules.  The hook renumbers the copy
//! (`id += 100`), so the two drops of a copied value are told apart: `D101` is the copy's,
//! `D1` the original's.  A trace that lost a drop, doubled one, or ran the hook on a move reads
//! differently, and each cell runs on BOTH backends.
//!
//! These are subprocess cells for the reason `tests/lease_refuse.rs` gives: the refusal switch is
//! cached per process.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

const PRELUDE: &str = "struct H { id: integer }\n\
                       fn OpDrop(self: H) { println(\"D{self.id}\"); }\n\
                       fn OpCopy(self: H) { println(\"C{self.id}\"); self.id += 100; }\n\
                       fn mk(i: integer) -> H { H { id: i } }\n\
                       struct S { h: H, n: integer }\n";

/// Run `body` after the prelude in `mode`; answer the program's lines joined by spaces, or the
/// first `error[code]` when it does not compile.
fn run(tag: &str, body: &str, mode: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "loft_copy_lease_{tag}_{}_{}.loft",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, format!("{PRELUDE}{body}\n")).expect("write cell");
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg(mode)
        .arg(&path)
        .env("LOFT_TIMEOUT", "240")
        .output()
        .expect("run loft");
    let _ = std::fs::remove_file(&path);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if let Some(at) = text.find("error[") {
        let code = &text[at..];
        return code[..code.find(']').map_or(code.len(), |e| e + 1)].to_string();
    }
    text.lines()
        .filter(|l| {
            l.starts_with(['R', 'D', 'C', 'b'])
                && !l.starts_with("Dead")
                && !l.starts_with("Consider")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn check(cells: &[(&str, &str, &str)]) {
    let mut wrong = Vec::new();
    for (tag, body, want) in cells {
        for mode in ["--interpret", "--native"] {
            let got = run(tag, body, mode);
            if got != *want {
                wrong.push(format!("{tag} {mode}: got `{got}`, want `{want}`"));
            }
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

/// Each copy the census can follow with a call: the hook runs on the new structure, each
/// structure drops once.
#[test]
fn a_copy_of_a_leasing_type_runs_its_hook_and_drops_twice() {
    check(&[
        (
            "param_bind",
            "fn keep(p: H) { x = p; println(\"R{x.id}\"); }\n\
             fn main() { a = mk(1); keep(a); println(\"back {a.id}\"); }",
            "C1 R101 D101 back 1 D1",
        ),
        (
            "member_in_literal",
            "fn main() { s = S { h: mk(1), n: 0 }; t = S { h: s.h, n: 1 }; \
             println(\"R{t.h.id} {s.h.id}\"); }",
            "C1 R101 1 D101 D1",
        ),
        (
            "member_appended",
            "fn main() { s = S { h: mk(1), n: 0 }; v: vector<H> = []; v += [s.h]; \
             println(\"R{v[0].id} {s.h.id}\"); }",
            "C1 R101 1 D101 D1",
        ),
        (
            "coalesce_arm",
            "fn pick(p: H?) { x = p ?? mk(9); println(\"R{x.id}\"); }\n\
             fn main() { a = mk(1); pick(a); println(\"back {a.id}\"); }",
            "C1 R101 D101 back 1 D1",
        ),
        (
            "struct_cascade",
            "fn cp(p: S) { s2 = p; println(\"R{s2.h.id}\"); }\n\
             fn main() { s = S { h: mk(1), n: 0 }; cp(s); println(\"back {s.h.id}\"); }",
            "C1 R101 D101 back 1 D1",
        ),
        (
            "enum_cascade",
            "enum W { WH { h: H }, WNone }\n\
             fn cp(p: W) { w2 = p; match w2 { WH { h } => println(\"R{h.id}\"), WNone => {} } }\n\
             fn main() { w = WH { h: mk(1) }; cp(w); println(\"back\"); }",
            "C1 R101 D101 back D1",
        ),
        // The container's own hook first, then its members' — for the copy as for the drop.
        (
            "own_hook_first",
            "struct S2 { h: H, n: integer }\n\
             fn OpDrop(self: S2) { println(\"DS{self.n}\"); }\n\
             fn OpCopy(self: S2) { println(\"CS{self.n}\"); self.n += 100; }\n\
             fn cp(p: S2) { x = p; println(\"R{x.n} {x.h.id}\"); }\n\
             fn main() { s = S2 { h: mk(1), n: 5 }; cp(s); println(\"back\"); }",
            "CS5 C1 R105 101 DS105 D101 back DS5 D1",
        ),
    ]);
}

/// What is not a copy runs no hook: a view, a move, an argument, a rebind's displaced value,
/// and a vector's growth after a copy — `(H-Lease)`'s "moving bytes without making a structure
/// is not a copy".
#[test]
fn what_is_not_a_copy_runs_no_hook() {
    check(&[
        (
            "view",
            "fn main() { s = S { h: mk(1), n: 0 }; x = s.h; println(\"R{x.id}\"); }",
            "R1 D1",
        ),
        (
            "move",
            "fn main() { a = mk(1); b = a; println(\"R{b.id}\"); }",
            "R1 D1",
        ),
        (
            "argument",
            "fn show(p: H) { println(\"R{p.id}\"); }\n\
             fn main() { a = mk(1); show(a); println(\"back {a.id}\"); }",
            "R1 back 1 D1",
        ),
        (
            "rebind",
            "fn main() { a = mk(1); a = mk(2); println(\"R{a.id}\"); }",
            "D1 R2 D2",
        ),
        (
            "growth",
            "fn main() { s = S { h: mk(1), n: 0 }; v: vector<H> = []; v += [s.h]; \
             for i in 2..5 { v += [mk(i * 10)]; } println(\"R{v[0].id} {len(v)}\"); }",
            "C1 R101 4 D101 D20 D30 D40 D1",
        ),
    ]);
}

/// The copies the lowering would otherwise make a VIEW of are materialised, so they lease too:
/// the identity `p = p` of a parameter, a tuple item, a `return` of a parameter or a member (as a
/// statement, as a tail, and per arm of a join — the fresh arm runs no hook), and a whole
/// collection copied or appended, which leases only the appended elements.  A copy the compiler
/// ELIDES together with its drop runs nothing, which is `(H-Elide)`.
#[test]
fn a_copy_the_lowering_would_view_is_materialised_and_leases() {
    check(&[
        (
            "self_rebind",
            "fn keep(p: H) { p = p; println(\"R{p.id}\"); }\n\
             fn main() { a = mk(1); keep(a); println(\"back {a.id}\"); }",
            "C1 R101 D101 back 1 D1",
        ),
        (
            "tuple_items",
            "fn tup(p: H) { t = (p, 1); println(\"R{t.0.id}\"); }\n\
             fn main() { a = mk(1); tup(a); println(\"back {a.id}\"); \
             s = S { h: mk(2), n: 0 }; u = (s.h, 5); println(\"R{u.0.id} {s.h.id}\"); }",
            "C1 R101 D101 back 1 C2 R102 2 D102 D2 D1",
        ),
        (
            "returns",
            "fn same(p: H) -> H { return p; }\n\
             fn tail(p: H) -> H { p }\n\
             fn inner(s: S) -> H { return s.h; }\n\
             fn own() -> H { a = mk(9); a }\n\
             fn main() { a = mk(1); b = same(a); println(\"R{b.id} {a.id}\"); \
             c = tail(a); println(\"R{c.id}\"); \
             s = S { h: mk(2), n: 0 }; d = inner(s); println(\"R{d.id} {s.h.id}\"); \
             e = own(); println(\"R{e.id}\"); }",
            "C1 R101 1 C1 R101 C2 R102 2 R9 D9 D102 D2 D101 D101 D1",
        ),
        (
            "join_return",
            "fn pick(p: H?) -> H { return p ?? mk(9); }\n\
             fn main() { a = mk(1); b = pick(a); println(\"R{b.id} {a.id}\"); \
             c = pick(null); println(\"R{c.id}\"); }",
            "C1 R101 1 R9 D9 D101 D1",
        ),
        (
            "collection_copy",
            "fn cp(p: vector<H>) { u = p; u += [mk(3)]; \
             println(\"R{len(u)} {u[0].id} {u[1].id} {u[2].id}\"); }\n\
             fn main() { w: vector<H> = [mk(1), mk(2)]; cp(w); println(\"back {w[0].id}\"); }",
            "C1 C2 R3 101 102 3 D101 D102 D3 back 1 D1 D2",
        ),
        (
            "collection_append",
            "fn cat(p: vector<H>) { v: vector<H> = [mk(7)]; v += p; \
             println(\"R{len(v)} {v[0].id} {v[1].id} {v[2].id}\"); }\n\
             fn main() { w: vector<H> = [mk(1), mk(2)]; cat(w); println(\"back {w[0].id}\"); }",
            "C1 C2 R3 7 101 102 D7 D101 D102 back 1 D1 D2",
        ),
        (
            "collection_elided",
            "fn cp(p: vector<H>) { u = p; println(\"R{len(u)} {u[0].id}\"); }\n\
             fn main() { w: vector<H> = [mk(1), mk(2)]; cp(w); println(\"back {w[0].id}\"); }",
            "R2 1 back 1 D1 D2",
        ),
    ]);
}

/// A member without `OpCopy` still refuses the whole copy: two structures would share the member's
/// resource and release it twice.
#[test]
fn a_copy_that_cannot_lease_stays_refused() {
    check(&[(
        "member_without_hook",
        "struct K { id: integer }\n\
         fn OpDrop(self: K) { println(\"DK{self.id}\"); }\n\
         struct S3 { h: H, k: K }\n\
         fn cp(p: S3) { x = p; println(\"R{x.h.id}\"); }\n\
         fn main() { s = S3 { h: mk(1), k: K { id: 7 } }; cp(s); println(\"back\"); }",
        "error[copy-of-droppable]",
    )]);
}
