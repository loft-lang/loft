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
    run_env(tag, body, mode, &[])
}

fn run_env(tag: &str, body: &str, mode: &str, env: &[(&str, &str)]) -> String {
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
        .envs(env.iter().copied())
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
        // A member of a tuple PARAMETER bound to a name is a view too (`(B-View)`); it once rode
        // a reverse hand-off @PLN163 P5 removed, so it is pinned here.
        (
            "tuple_param_view",
            "fn view(t: (H, integer)) { x = t.0; println(\"R{x.id} {t.1}\"); }\n\
             fn main() { a = (mk(1), 5); view(a); println(\"back {a.0.id}\"); }",
            "R1 5 back 1 D1",
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

/// A MEMBER copied out of its owner leases a structure of its own, so the owner still releases the
/// member it keeps: the call result's record at the end of its statement, a returned member's local
/// at its function's end.  Their release used to be handed to the copy (`OpDropAllExcept`), which
/// lost one of the two — @PLN163 P6 removed that hand-off.  A read THROUGH a member of a call
/// result is a view of the call's record, with no copy and no hook.
#[test]
fn a_member_copied_out_leases_and_its_owner_keeps_its_release() {
    check(&[
        (
            "call_member",
            "fn mk_s(i: integer) -> S { S { h: mk(i), n: 0 } }\n\
             fn main() { r = mk_s(1).h; println(\"R{r.id}\"); println(\"R{mk_s(2).h.id}\"); }",
            "C1 D1 R101 R2 D2 D101",
        ),
        (
            "argument_view",
            "fn mk_s(i: integer) -> S { S { h: mk(i), n: 0 } }\n\
             fn take(h: H) -> integer { h.id }\n\
             fn main() { println(\"R{take(mk_s(1).h)}\"); }",
            "R1 D1",
        ),
        (
            "returned_member",
            "fn f() -> H { s = S { h: mk(1), n: 0 }; return s.h; }\n\
             fn main() { r = f(); println(\"R{r.id}\"); }",
            "C1 D1 R101 D101",
        ),
    ]);
}

/// The RETURN of a struct-enum leases like a record's (found by loft2-d9: the enum arm of
/// `parse_return` never asked the leasing question, so `return p` was refused as a copy the
/// caller still owns), and so does a value returned from a LAMBDA: its own capture, a local copy
/// of that capture — built in a work-ref the local views, and handed to the reserved `__retbuf`
/// as a move — and a local copy of a PARAMETER, which holds a leased structure of its own rather
/// than the caller's record.
#[test]
fn an_enum_or_lambda_return_leases_like_a_record() {
    const E: &str = "enum EL { EL1 { h: H }, EL0 }\n\
                     fn rd(e: EL) -> integer { match e { EL1 { h } => h.id, EL0 => 0 } }\n";
    let enum_cells: Vec<(String, String, &str)> = vec![
        (
            "e_stmt",
            "fn e(p: EL) -> EL { return p; }\n\
          fn main() { a = EL1 { h: mk(1) }; b = e(a); println(\"R{rd(b)} {rd(a)}\"); }",
            "C1 R101 1 D101 D1",
        ),
        (
            "e_tail",
            "fn e(p: EL) -> EL { p }\n\
          fn main() { a = EL1 { h: mk(2) }; b = e(a); println(\"R{rd(b)} {rd(a)}\"); }",
            "C2 R102 2 D102 D2",
        ),
        (
            "e_member",
            "struct SW { e: EL }\nfn e(s: SW) -> EL { return s.e; }\n\
          fn main() { s = SW { e: EL1 { h: mk(3) } }; b = e(s); println(\"R{rd(b)} {rd(s.e)}\"); }",
            "C3 R103 3 D103 D3",
        ),
        (
            "e_join",
            "fn e(p: EL, c: boolean) -> EL { if c { p } else { EL1 { h: mk(9) } } }\n\
          fn main() { a = EL1 { h: mk(4) }; b = e(a, true); println(\"R{rd(b)}\"); \
          c = e(a, false); println(\"R{rd(c)}\"); }",
            "C4 R104 R9 D9 D104 D4",
        ),
        (
            "e_unit",
            "fn e(p: EL) -> EL { return p; }\n\
          fn main() { a: EL = EL0; b = e(a); println(\"R{rd(b)}\"); }",
            "R0",
        ),
        (
            "l_capture",
            "fn main() { c = mk(1); f = fn() -> H { c }; a = f(); println(\"R{a.id}\"); \
          b = f(); println(\"R{b.id}\"); }",
            "C1 R101 C1 R101 D101 D101 D1",
        ),
        (
            "l_capture_local",
            "fn main() { c = mk(1); f = fn() -> H { x = c; x }; a = f(); \
          println(\"R{a.id}\"); }",
            "C1 R101 D101 D1",
        ),
        (
            "l_enum_capture_local",
            "fn main() { c = EL1 { h: mk(2) }; f = fn() -> EL { x = c; x }; \
          a = f(); println(\"R{rd(a)}\"); }",
            "C2 R102 D102 D2",
        ),
        (
            "l_owned",
            "fn main() { f = fn() -> H { x = mk(7); x }; a = f(); println(\"R{a.id}\"); }",
            "R7 D7",
        ),
        (
            "param_local",
            "fn g(c: H) -> H { x = c; x }\n\
          fn main() { a = mk(1); b = g(a); println(\"R{b.id} {a.id}\"); }",
            "C1 R101 1 D101 D1",
        ),
    ]
    .into_iter()
    .map(|(t, b, w)| (t.to_string(), format!("{E}{b}"), w))
    .collect();
    let cells: Vec<(&str, &str, &str)> = enum_cells
        .iter()
        .map(|(t, b, w)| (t.as_str(), b.as_str(), *w))
        .collect();
    check(&cells);
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

/// `(H-Elide)` — the compiler MAY elide a copy of a leasing type together with the drop of the
/// structure it would have made, and neither hook may rely on running for an elided copy
/// (@PLN163 P6).  Every route that elides a copy is asked, with the transparent-link widening
/// (`LOFT_LINK_WIDEN`, the one switch that widens elision) OFF and ON, on both backends, and each
/// must print the trace written here by hand: where the copy is KEPT its `OpCopy` runs and both
/// structures drop once, where it is ELIDED (a read through a parameter's member) neither hook
/// runs for it, and the widening changes no trace.  Measured 2026-09-25: it elides no leasing copy
/// today — the rule is permissive, so that is compliance, and this cell holds it there.
#[test]
fn an_elided_copy_skips_its_hook_and_its_drop_together() {
    let cells: &[(&str, &str, &str)] = &[
        (
            "param",
            "fn f(p: H) { u = p; println(\"R{u.id}\"); }\nfn main() { a = mk(1); f(a); }",
            "C1 R101 D101 D1",
        ),
        (
            "member_read",
            "fn h(o: S) { u = o.h; println(\"R{u.id}\"); }\nfn main() { s = S { h: mk(3), n: 0 }; h(s); }",
            "R3 D3",
        ),
        (
            "chain",
            "fn f(p: H) { u = p; v = u; println(\"R{v.id}\"); }\nfn main() { a = mk(5); f(a); }",
            "C5 R105 D105 D5",
        ),
        (
            "ret_field",
            "fn f(p: H) -> integer { u = p; u.id }\nfn main() { a = mk(8); x = f(a); println(\"R{x}\"); }",
            "C8 D108 R108 D8",
        ),
        (
            "nested",
            "fn g(q: H) -> integer { w = q; w.id }\nfn f(p: H) -> integer { u = p; g(u) }\nfn main() { a = mk(9); println(\"R{f(a)}\"); }",
            "C9 C109 D209 D109 R209 D9",
        ),
    ];
    let mut wrong = Vec::new();
    for (tag, body, want) in cells {
        for mode in ["--interpret", "--native"] {
            for widen in [&[][..], &[("LOFT_LINK_WIDEN", "1")][..]] {
                let got = run_env(tag, body, mode, widen);
                if got != *want {
                    wrong.push(format!(
                        "{tag} {mode} widen={}: got `{got}`, want `{want}`",
                        !widen.is_empty()
                    ));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}
