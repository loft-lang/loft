// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! The `(H-Drop)` gate: every droppable resource is released exactly once, at the death of the
//! record that owns it, and the places `(H-Drop-Not)` names release nothing (@FR-H-Drop,
//! @FR-H-Drop-Not).
//!
//! **Why a gate of its own.** The free-side instruments cannot see a drop.  `LOFT_POISON`, the
//! leak check and the ownership oracle's Check D all ask whether a FREE is sound, and all of
//! them stay clean on a program that runs a hook twice or never.  A drop also has no safe
//! direction to err in: a hook that does not run leaves a handle open, and a hook that runs
//! twice closes a handle the author already closed.  So this gate scores the release itself.
//!
//! **The oracle is one comparison.** Every cell is a small program whose resource type `H`
//! prints `M<id>` when it is minted, `D<id>` when its hook runs, and `R<id>` when the program
//! reads it; `X<id>` marks a resource `(H-Drop-Not)` makes the author's to release.  A copy
//! carries the id, so an id is a lineage, and the rule becomes a fact about the trace:
//!
//! - each minted id is released exactly once — or never, once `X<id>` was printed;
//! - no id is released that was never minted (a hook that read freed memory);
//! - no id is released before a later read of it (released while still in use);
//! - no id is released after its cell ended (released by something other than its owner).
//!
//! The generator never computes an expectation, so it cannot drift from the scorer: the
//! program reports what happened and [`score`] is the only place the rule is written down.
//!
//! **Each cell runs alone.** A release that runs twice corrupts what later code sees — a cell
//! that is clean on its own lost its release when it ran after a doubling one.  So a batch
//! cannot be scored cell by cell, and every cell is its own process.  A cell that the compiler
//! refuses, or that crashes, gets a verdict of its own and is never scored as clean.
//!
//! **The cells** come from three families.  The PILOT cells are written by hand: controls for
//! shapes that are known to hold, the open `heap.md` D-heap-1 shapes, and the `(H-Drop-Not)`
//! boundary.  The CROSS family composes nine SOURCE spellings of a resource (a fresh call, a
//! local, a field, an element, a tuple member, a parameter, a parameter's field, a call
//! result's field, a `??`) with eleven DESTINATIONS it is copied into.  The COALESCE family
//! varies `A ?? B` along the path taken, the spelling of `A`, the spelling of `B` and the
//! destination.
//!
//! **The baseline** (`tests/ownership_drop_gate.baseline`) lists every cell that is not clean,
//! with the kinds of finding it has.  It is a record of the tree it was measured on: the test
//! fails when a cell gains or changes a finding, and it also fails when a finding is GONE, so
//! that a fix retires its line in the same commit.  A line in it is an open defect, not an
//! accepted behaviour — the register for each one is `doc/claude/formal/heap.md`.
//!
//!   run:    cargo test --release --test ownership_drop_gate      (both backends)
//!   bless:  LOFT_BLESS_DROP_GATE=1 cargo test --release --test ownership_drop_gate
//!           (writes `ownership_drop_gate.baseline` and `ownership_drop_gate.native.baseline`)

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The resource type and the containers the cells copy it into.  Every cell program starts
/// with this text.
const PRELUDE: &str = r#"struct H { id: integer }
struct S { h: H }
struct Hold { h: H }
struct SN { h: H? }
enum W { WH { h: H }, WNone }
struct Bag { v: vector<H>, tag: integer }
struct K { key: integer, h: H }
struct KBox { hs: hash<K[key]> }
fn OpDrop(self: H) { println("D{self.id}"); }
fn mk(id: integer) -> H { println("M{id}"); return H { id: id }; }
fn lit(id: integer) -> integer { println("M{id}"); return id; }
fn mk_s(id: integer) -> S { return S { h: mk(id) }; }
"#;

const BASELINE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/ownership_drop_gate.baseline"
);
const NATIVE_BASELINE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/ownership_drop_gate.native.baseline"
);
const BLESS: &str = "LOFT_BLESS_DROP_GATE";

/// One generated program: `text` defines a function named `name` (and its helpers), which
/// `main` calls between the `C<name>` and `Cend` markers.
struct Cell {
    name: String,
    text: String,
}

fn cell(name: &str, text: &str) -> Cell {
    Cell {
        name: name.to_string(),
        text: text.to_string(),
    }
}

fn program(c: &Cell) -> String {
    format!(
        "{PRELUDE}\n{}\nfn main() {{ println(\"C{}\"); {}(); println(\"Cend\"); }}\n",
        c.text, c.name, c.name
    )
}

// ── the PILOT family ────────────────────────────────────────────────────────────────────────

fn pilot_cells() -> Vec<Cell> {
    vec![
        // Controls: shapes the rule is known to hold for.
        cell(
            "p_k1",
            r#"fn p_k1() { h = mk(1); h2 = h; println("R{h2.id}"); }"#,
        ),
        cell(
            "p_k2",
            r#"fn p_k2() { s = S { h: mk(2) }; t = s; println("R{t.h.id}"); }"#,
        ),
        cell(
            "p_k3",
            r#"fn p_k3_m() -> S { s = S { h: mk(3) }; t = s; return t; }
fn p_k3() { r = p_k3_m(); println("R{r.h.id}"); }"#,
        ),
        cell(
            "p_k4",
            r#"fn p_k4_b(k: integer) { s: S = if k == 0 { a: S = S { h: mk(4) }; a } else { b: S = S { h: mk(5) }; b }; println("R{s.h.id}"); }
fn p_k4() { p_k4_b(0); }"#,
        ),
        cell(
            "p_k5",
            r#"fn p_k5() { v: vector<S> = []; v += [S { h: mk(6) }]; println("R{v[0].h.id}"); }"#,
        ),
        cell(
            "p_k6",
            r#"fn p_k6() { b = Bag { v: [mk(7), mk(8)], tag: 1 }; println("R{b.v[1].id}"); }"#,
        ),
        cell(
            "p_k7",
            r#"fn p_k7_b(x: S) { t = x; println("R{t.h.id}"); }
fn p_k7() { s = S { h: mk(9) }; p_k7_b(s); println("R{s.h.id}"); }"#,
        ),
        // heap.md D-heap-1's open shapes.
        cell(
            "p_o1",
            r#"fn p_o1() { v: vector<(H, integer)> = [(mk(10), 1)]; for e in v { u = e; println("R{u.0.id}"); } }"#,
        ),
        cell(
            "p_o2",
            r#"fn p_o2() { t = (mk(11), 1); u = t; t = (mk(12), 2); z = t; println("R{u.0.id}"); println("R{z.0.id}"); }"#,
        ),
        cell(
            "p_o3",
            r#"fn p_o3_m() -> H { t = (mk(13), 1); x = t.0; return x; }
fn p_o3() { r = p_o3_m(); println("R{r.id}"); }"#,
        ),
        cell(
            "p_o4",
            r#"fn p_o4_m(d: H) -> H { t: (H?, integer) = (mk(14), 1); return t.0 ?? d; }
fn p_o4() { d = mk(15); r = p_o4_m(d); println("R{r.id}"); }"#,
        ),
        cell(
            "p_o5",
            r#"fn p_o5_m(p: (H, integer)) { c = Hold { h: p.0 }; println("R{c.h.id}"); }
fn p_o5() { t = (mk(16), 1); p_o5_m(t); println("R{t.0.id}"); }"#,
        ),
        // A `??` result bound inside a loop body: the arm lift temp is Set again on every
        // iteration, on both paths.
        cell(
            "p_l1",
            r#"fn p_l1() { a: H? = mk(24); for i in 0..2 { x = a ?? mk(25 + i); println("R{x.id}"); } }"#,
        ),
        cell(
            "p_l2",
            r#"fn p_l2() { a: H? = null; for i in 0..2 { x = a ?? mk(27 + i); println("R{x.id}"); } }"#,
        ),
        // The same joins spelled as a value `if`, beside their `??` spellings in the COALESCE
        // family: `a ?? d` is that `if`, so the two must answer alike.
        cell(
            "p_j1",
            r#"fn p_j1() { a: H? = mk(30); x = mk(31); x = if a != null { a } else { mk(32) }; println("R{x.id}"); }"#,
        ),
        cell(
            "p_j2",
            r#"fn p_j2() { a: H? = null; x = mk(33); x = if a != null { a } else { mk(34) }; println("R{x.id}"); }"#,
        ),
        cell(
            "p_j3",
            r#"fn p_j3() { a: H? = mk(35); v: vector<H> = []; v += [if a != null { a } else { mk(36) }]; println("R{v[0].id}"); }"#,
        ),
        cell(
            "p_j4",
            r#"fn p_j4() { a: H? = null; v: vector<H> = []; v += [if a != null { a } else { mk(37) }]; println("R{v[0].id}"); }"#,
        ),
        // A reassigned local whose value is handed off by a LATER statement, or off a
        // parameter: a hand-off belongs to the assignment it follows, so it must not suppress
        // the release of a record an earlier assignment gave the local.
        cell(
            "p_h1",
            r#"fn p_h1() { b = mk(40); b = mk(41); y = b; println("R{y.id}"); }"#,
        ),
        cell(
            "p_h2",
            r#"fn p_h2_b(p: H) { x = mk(42); x = p; println("R{x.id}"); }
fn p_h2() { a = mk(43); p_h2_b(a); println("R{a.id}"); }"#,
        ),
        cell(
            "p_h3",
            r#"fn p_h3_b(p: H) { x = mk(44); x = p; x = mk(45); println("R{x.id}"); }
fn p_h3() { a = mk(46); p_h3_b(a); println("R{a.id}"); }"#,
        ),
        cell(
            "p_h4",
            r#"fn p_h4_b(p: H) { x = p; x = mk(47); println("R{x.id}"); }
fn p_h4() { a = mk(48); p_h4_b(a); println("R{a.id}"); }"#,
        ),
        cell(
            "p_h5",
            r#"fn p_h5_b(p: H, c: boolean) { x = mk(49); if c { x = p; } println("R{x.id}"); }
fn p_h5() { a = mk(50); p_h5_b(a, true); println("R{a.id}"); }"#,
        ),
        cell(
            "p_h6",
            r#"fn p_h6_b(p: H, c: boolean) { x = mk(51); if c { x = p; } println("R{x.id}"); }
fn p_h6() { a = mk(52); p_h6_b(a, false); println("R{a.id}"); }"#,
        ),
        cell(
            "p_h7",
            r#"fn p_h7_b(p: H) { x = mk(53); for i in 0..2 { x = mk(54 + i); x = p; } println("R{x.id}"); }
fn p_h7() { a = mk(56); p_h7_b(a); println("R{a.id}"); }"#,
        ),
        // An element copied into a second vector while the element's id is SMALL: the id is
        // the value an uninitialised store slot would hold, so a release snapshot read ahead
        // of its declaration lands in bounds and runs the hook over garbage records instead of
        // crashing (the CROSS family's `c_elem_push` spells the crashing, large-id face).
        cell(
            "p_e1",
            r#"fn p_e1() { vs: vector<H> = [mk(1)]; v: vector<H> = []; v += [vs[0]]; println("R{v[0].id}"); }"#,
        ),
        cell(
            "p_e2",
            r#"fn p_e2() { vs: vector<H> = [mk(3)]; v: vector<H> = []; v += [vs[0]]; println("R{v[0].id}"); }"#,
        ),
        // Droppable-element vectors built one after another, each dead before the next one is
        // built: the shape last-use reclaim frees early.  Each element is released once, by its
        // own vector's release.
        cell(
            "p_r1",
            r#"fn p_r1() {
  a: vector<H> = [mk(60)]; println("R{a[0].id}");
  b: vector<H> = [mk(61)]; println("R{b[0].id}");
  c: vector<H> = [mk(62), mk(63)]; println("R{c[1].id}");
}"#,
        ),
        // A construction delivered through a join ARM into a local: the arm's work-ref hands
        // the record to the binding on the path that ran and holds nothing on the others.  A
        // struct literal arm is the plain spelling; `match`, a loop and a rebind are the others.
        cell(
            "p_g1",
            r#"fn p_g1_b(k: integer) { x: H = if k > 0 { mk_s(64).h } else { mk_s(65).h }; println("R{x.id}"); }
fn p_g1() { p_g1_b(1); p_g1_b(0); }"#,
        ),
        cell(
            "p_g2",
            r#"fn p_g2_b(k: integer) { x: H = if k > 0 { H { id: lit(66) } } else { mk(67) }; println("R{x.id}"); }
fn p_g2() { p_g2_b(1); }"#,
        ),
        cell(
            "p_g3",
            r#"fn p_g3_b(k: integer) { x: H = match k { 1 => mk_s(68).h, _ => mk(69) }; println("R{x.id}"); }
fn p_g3() { p_g3_b(1); }"#,
        ),
        cell(
            "p_g4",
            r#"fn p_g4() { for i in 0..4 { x: H = if i % 2 == 0 { mk_s(70 + i).h } else { mk(80 + i) }; println("R{x.id}"); } }"#,
        ),
        cell(
            "p_g5",
            r#"fn p_g5_b(k: integer) { x: H = mk(90); x = if k > 0 { mk_s(91).h } else { mk(92) }; println("R{x.id}"); }
fn p_g5() { p_g5_b(1); }"#,
        ),
        // A reassignment from a join whose arms are two owned locals: the displaced record and
        // the untaken local are each still released once, on either path.
        cell(
            "p_s1",
            r#"fn p_s1_b(c: boolean) { a = mk(94); b = mk(95); x = mk(96); x = if c { a } else { b }; println("R{x.id}"); }
fn p_s1() { p_s1_b(true); }"#,
        ),
        cell(
            "p_s2",
            r#"fn p_s2_b(c: boolean) { a = mk(97); b = mk(98); x = mk(99); x = if c { a } else { b }; println("R{x.id}"); }
fn p_s2() { p_s2_b(false); }"#,
        ),
        // The same join written as a statement by the author.
        cell(
            "p_s3",
            r#"fn p_s3_b(c: boolean) { a = mk(100); b = mk(101); x = mk(102); if c { x = a; } else { x = b; } println("R{x.id}"); }
fn p_s3() { p_s3_b(true); }"#,
        ),
        // A per-path copy of `a` that did not run, then an unconditional one: `a`'s resource is
        // `y`'s alone.
        cell(
            "p_s4",
            r#"fn p_s4_b(c: boolean) { a = mk(103); b = mk(104); x = mk(105); if c { x = a; } else { x = b; } y = a; println("R{x.id} Y{y.id}"); }
fn p_s4() { p_s4_b(false); }"#,
        ),
        // A per-path copy of `a` that did not run, then `a` is rebound: the record it displaces is
        // released there.
        cell(
            "p_s5",
            r#"fn p_s5_b(c: boolean) { a = mk(106); b = mk(107); x = mk(108); if c { x = a; } else { x = b; } a = mk(109); println("R{x.id} A{a.id}"); }
fn p_s5() { p_s5_b(false); }"#,
        ),
        // A reassignment from `??` with a LOCAL default: the subject present, so the default's
        // own record is still its local's to release.
        cell(
            "p_s6",
            r#"fn p_s6() { a: H? = mk(110); b = mk(111); x = mk(112); x = a ?? b; println("R{x.id}"); }"#,
        ),
        // The same with the subject absent: the local keeps a copy of the default, which
        // releases once.
        cell(
            "p_s7",
            r#"fn p_s7() { a: H? = null; b = mk(113); x = mk(114); x = a ?? b; println("R{x.id}"); }"#,
        ),
        // A reassignment from a scalar `match` whose arms have no block of their own, repeated in
        // a loop: each iteration releases the record the previous one assigned.
        cell(
            "p_s8",
            r#"fn p_s8_b(k: integer) { x = mk(115); for i in 0..2 { x = match k { 0 => mk(116 + i), _ => mk(118 + i) }; println("R{x.id}"); } }
fn p_s8() { p_s8_b(0); }"#,
        ),
        // A variable rebound to its own value: `a = a` and `a = a ?? d` each WRITE a copy of the
        // existing `a`, so both are refused (`formal/heap.md` H-Copy-Refuse; heap.md D-heap-10
        // records how the second released twice while it compiled).
        cell(
            "p_i1",
            r#"fn p_i1() { a = mk(120); a = a; println("R{a.id}"); }"#,
        ),
        cell(
            "p_i2",
            r#"fn p_i2() { a: H? = mk(121); a = a ?? mk(122); println("R{a.id}"); }"#,
        ),
        // A member read through a call result, further along the chain: the compiler copies the
        // member only to read `.id` off it, and nothing outlives the expression, so the line
        // writes no copy (`formal/heap.md` H-Move).  The liveness reading refused it.
        cell("p_t1", r#"fn p_t1() { println("R{mk_s(130).h.id}"); }"#),
        // (H-Drop-Not): the language releases nothing for the X-marked id.
        cell(
            "p_n1",
            r#"fn p_n1() { s = S { h: mk(17) }; s.h = mk(18); println("X17"); println("R{s.h.id}"); }"#,
        ),
        cell(
            "p_n2",
            r#"fn p_n2() { v: vector<H> = [mk(19)]; v[0] = mk(20); println("X19"); println("R{v[0].id}"); }"#,
        ),
        cell(
            "p_n3",
            r#"fn p_n3() { v: vector<H> = [mk(21), mk(22)]; v.remove(0); println("X21"); println("R{v[0].id}"); }"#,
        ),
        cell(
            "p_n4",
            r#"fn p_n4() { b = KBox { hs: [] }; b.hs += [K { key: 1, h: mk(23) }]; println("X23"); println("R{b.hs[1].h.id}"); }"#,
        ),
    ]
}

// ── the CROSS family: source spelling × destination ────────────────────────────────────────

/// `(name, params, setup, expr, caller_setup, caller_args, caller_after)`.  `@I` and `@J` are
/// the cell's ids.  A PARAMETER source is minted by the caller, which stays its owner and
/// reads it back after the call.
const SOURCES: &[(&str, &str, &str, &str, &str, &str, &str)] = &[
    ("call", "", "", "mk(@I)", "", "", ""),
    ("local", "", "a = mk(@I);", "a", "", "", ""),
    ("field", "", "s = S { h: mk(@I) };", "s.h", "", "", ""),
    ("elem", "", "vs: vector<H> = [mk(@I)];", "vs[0]", "", "", ""),
    ("tuple", "", "tt = (mk(@I), 1);", "tt.0", "", "", ""),
    (
        "param",
        "p: H",
        "",
        "p",
        "a = mk(@I);",
        "a",
        r#"println("R{a.id}");"#,
    ),
    (
        "pfield",
        "p: S",
        "",
        "p.h",
        "a = S { h: mk(@I) };",
        "a",
        r#"println("R{a.h.id}");"#,
    ),
    ("callproj", "", "", "mk_s(@I).h", "", "", ""),
    ("coalesce", "", "a: H? = mk(@I);", "a ?? mk(@J)", "", "", ""),
    ("literal", "", "", "H { id: lit(@I) }", "", "", ""),
];

/// `(name, statement)` over `@E`, the source expression; `ret` returns it from a helper
/// instead, and `arm`/`arm0`/`reassign` mint their other value as `@K`.  `arm` takes the arm
/// holding the source and `arm0` the other one, so both paths of the join are measured.
const DESTS: &[(&str, &str)] = &[
    ("local", r#"x = @E; println("R{x.id}");"#),
    ("annot", r#"x: H = @E; println("R{x.id}");"#),
    ("nullable", r#"x: H? = @E; println("R{x.id}");"#),
    ("field", r#"c = Hold { h: @E }; println("R{c.h.id}");"#),
    (
        "enum",
        r#"w: W = WH { h: @E }; match w { WH { h } => println("R{h.id}"), WNone => {} }"#,
    ),
    (
        "push",
        r#"v: vector<H> = []; v += [@E]; println("R{v[0].id}");"#,
    ),
    ("veclit", r#"v: vector<H> = [@E]; println("R{v[0].id}");"#),
    ("tuplem", r#"t = (@E, 1); println("R{t.0.id}");"#),
    (
        "arm",
        r#"x: H = if k > 0 { @E } else { mk(@K) }; println("R{x.id}");"#,
    ),
    (
        "arm0",
        r#"x: H = if k > 0 { @E } else { mk(@K) }; println("R{x.id}");"#,
    ),
    ("reassign", r#"x = mk(@K); x = @E; println("R{x.id}");"#),
    ("ret", ""),
];

fn join_nonempty(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

/// The parameter NAMES of a `name: type` list, for forwarding them to a helper.
fn param_names(params: &str) -> String {
    params
        .split(',')
        .filter_map(|p| p.split(':').next())
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn with_ids(text: &str, base: u32) -> String {
    text.replace("@I", &(base + 1).to_string())
        .replace("@J", &(base + 2).to_string())
        .replace("@K", &(base + 3).to_string())
}

fn cross_cells() -> Vec<Cell> {
    let mut out = Vec::new();
    let mut idx = 0u32;
    for &(sname, params, setup, expr, csetup, cargs, cafter) in SOURCES {
        for &(dname, dest) in DESTS {
            idx += 1;
            let name = format!("c_{sname}_{dname}");
            let arm = dname.starts_with("arm");
            let all_params = join_nonempty(&[params, if arm { "k: integer" } else { "" }]);
            let k_arg = match dname {
                "arm" => "1",
                "arm0" => "0",
                _ => "",
            };
            let call_args = join_nonempty(&[cargs, k_arg]);
            let mut text = String::new();
            if dname == "ret" {
                let _ = writeln!(
                    text,
                    "fn {name}_m({all_params}) -> H {{ {setup} return {expr}; }}"
                );
                let _ = writeln!(
                    text,
                    "fn {name}_b({all_params}) {{ x = {name}_m({}); println(\"R{{x.id}}\"); }}",
                    param_names(&all_params)
                );
            } else {
                let body = dest.replace("@E", expr);
                let _ = writeln!(text, "fn {name}_b({all_params}) {{ {setup} {body} }}");
            }
            let _ = writeln!(
                text,
                "fn {name}() {{ {csetup} {name}_b({call_args}); {cafter} }}"
            );
            out.push(Cell {
                text: with_ids(&text, 1000 * idx),
                name,
            });
        }
    }
    out
}

// ── the COALESCE family: `A ?? B` ───────────────────────────────────────────────────────────

/// `(name, params, setup when present, setup when absent, expr, caller when present, caller
/// when absent)`.
const COAL_A: &[(&str, &str, &str, &str, &str, &str, &str)] = &[
    ("local", "", "a: H? = mk(@I);", "a: H? = null;", "a", "", ""),
    (
        "param",
        "a: H?",
        "",
        "",
        "a",
        "q: H? = mk(@I);",
        "q: H? = null;",
    ),
    (
        "field",
        "",
        "s = SN { h: mk(@I) };",
        "s = SN { h: null };",
        "s.h",
        "",
        "",
    ),
];
/// `(name, setup, expr)` of the default.
const COAL_B: &[(&str, &str, &str)] = &[
    ("call", "", "mk(@J)"),
    ("var", "b = mk(@J);", "b"),
    ("literal", "", "H { id: lit(@J) }"),
];
const COAL_DEST: &[(&str, &str)] = &[
    ("local", r#"x = @E; println("R{x.id}");"#),
    ("field", r#"c = Hold { h: @E }; println("R{c.h.id}");"#),
    ("tuplem", r#"t = (@E, 1); println("R{t.0.id}");"#),
    (
        "arm",
        r#"x: H = if k > 0 { @E } else { mk(@K) }; println("R{x.id}");"#,
    ),
    (
        "push",
        r#"v: vector<H> = []; v += [@E]; println("R{v[0].id}");"#,
    ),
    ("ret", ""),
];

fn coalesce_cells() -> Vec<Cell> {
    let mut out = Vec::new();
    let mut idx = 0u32;
    for path in ["present", "default"] {
        for &(aname, params, set_p, set_d, aexpr, call_p, call_d) in COAL_A {
            for &(bname, bsetup, bexpr) in COAL_B {
                for &(dname, dest) in COAL_DEST {
                    idx += 1;
                    let present = path == "present";
                    let setup_a = if present { set_p } else { set_d };
                    let csetup = if present { call_p } else { call_d };
                    let expr = format!("{aexpr} ?? {bexpr}");
                    let name = format!("q_{path}_{aname}_{bname}_{dname}");
                    let arm = dname == "arm";
                    let is_param = aname == "param";
                    let all_params = join_nonempty(&[params, if arm { "k: integer" } else { "" }]);
                    let args = join_nonempty(&[
                        if is_param { "q" } else { "" },
                        if arm { "1" } else { "" },
                    ]);
                    let mut text = String::new();
                    if dname == "ret" {
                        let _ = writeln!(
                            text,
                            "fn {name}_m({all_params}) -> H {{ {setup_a} {bsetup} return {expr}; }}"
                        );
                        let _ = writeln!(
                            text,
                            "fn {name}_b({all_params}) {{ x = {name}_m({}); println(\"R{{x.id}}\"); }}",
                            param_names(&all_params)
                        );
                    } else {
                        let body = dest.replace("@E", &expr);
                        let _ = writeln!(
                            text,
                            "fn {name}_b({all_params}) {{ {setup_a} {bsetup} {body} }}"
                        );
                    }
                    let _ = writeln!(text, "fn {name}() {{ {csetup} {name}_b({args}); }}");
                    out.push(Cell {
                        text: with_ids(&text, 1000 * idx),
                        name,
                    });
                }
            }
        }
    }
    out
}

fn all_cells() -> Vec<Cell> {
    let mut cells = pilot_cells();
    cells.extend(cross_cells());
    cells.extend(coalesce_cells());
    cells
}

// ── the oracle ──────────────────────────────────────────────────────────────────────────────

/// The findings of one trace, as kinds (what the baseline pins) plus a line of detail each.
///
/// A trace with no `C` marker never reached `main` — the compiler refused it — and one with no
/// `Cend` stopped inside the cell.  Neither is scored: a partial trace would read as findings,
/// or as clean, about code that never ran.
fn score(stdout: &str, stderr: &str) -> (BTreeSet<&'static str>, Vec<String>) {
    let mut kinds = BTreeSet::new();
    let mut detail = Vec::new();
    let first_problem = || {
        stderr
            .lines()
            .find(|l| l.contains("rror") || l.contains("panic"))
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let events: Vec<(char, &str)> = stdout
        .lines()
        .map(str::trim)
        .filter_map(|l| {
            let k = l.chars().next()?;
            let rest = &l[k.len_utf8()..];
            ("CMDRX".contains(k) && !rest.is_empty() && !rest.contains(char::is_whitespace))
                .then_some((k, rest))
        })
        .collect();
    if !events.iter().any(|&(k, v)| k == 'C' && v != "end") {
        kinds.insert("REFUSED");
        detail.push(format!("REFUSED: {}", first_problem()));
        return (kinds, detail);
    }
    if !events.iter().any(|&(k, v)| k == 'C' && v == "end") {
        kinds.insert("CRASHED");
        detail.push(format!("CRASHED: {}", first_problem()));
        return (kinds, detail);
    }
    let mut cell = "?";
    let mut minted: HashMap<&str, &str> = HashMap::new();
    let mut released: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut exempt: HashSet<&str> = HashSet::new();
    let mut early: BTreeMap<&str, &str> = BTreeMap::new();
    for &(k, v) in &events {
        match k {
            'C' => cell = v,
            'M' => {
                if minted.insert(v, cell).is_some() {
                    kinds.insert("REMINT");
                    detail.push(format!("REMINT id {v}: minted twice — the ids collide"));
                }
            }
            'D' => released.entry(v).or_default().push(cell),
            'X' => {
                exempt.insert(v);
            }
            _ => {
                if released.contains_key(v) {
                    early.entry(v).or_insert(cell);
                }
            }
        }
    }
    if minted.is_empty() {
        kinds.insert("EMPTY");
        detail.push("EMPTY: the cell minted nothing, so it tested nothing".to_string());
    }
    let mut ids: Vec<&&str> = minted.keys().collect();
    ids.sort();
    for id in ids {
        let n = released.get(*id).map_or(0, Vec::len);
        let want = usize::from(!exempt.contains(*id));
        if n != want {
            let kind = if n < want {
                "LOST"
            } else if want == 0 {
                "RELEASED_NOT"
            } else {
                "DOUBLE"
            };
            kinds.insert(kind);
            detail.push(format!("{kind} id {id}: released {n}x, want {want}"));
        }
    }
    for (id, cells) in &released {
        match minted.get(id) {
            None => {
                kinds.insert("UNMINTED");
                detail.push(format!(
                    "UNMINTED id {id}: released {}x, never minted",
                    cells.len()
                ));
            }
            Some(owner) if cells.len() == 1 && cells[0] != *owner => {
                kinds.insert("LATE");
                detail.push(format!("LATE id {id}: released after its cell ended"));
            }
            Some(_) => {}
        }
    }
    for (id, at) in &early {
        if minted.contains_key(id) {
            kinds.insert("EARLY");
            detail.push(format!("EARLY id {id}: released before a read in {at}"));
        }
    }
    (kinds, detail)
}

struct Verdict {
    name: String,
    kinds: BTreeSet<&'static str>,
    detail: Vec<String>,
}

// ── running ─────────────────────────────────────────────────────────────────────────────────

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn run_cell(dir: &Path, c: &Cell, mode: &str, timeout: &str) -> Verdict {
    let path = dir.join(format!("{}.loft", c.name));
    std::fs::write(&path, program(c)).unwrap_or_else(|e| panic!("write {}: {e}", c.name));
    let mut cmd = Command::new(loft_bin());
    cmd.arg(mode)
        .arg(&path)
        .current_dir(dir)
        .env("LOFT_TIMEOUT", timeout);
    cap_address_space(&mut cmd, mode);
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("spawn loft {mode} for {}: {e}", c.name));
    let (kinds, detail) = score(
        &String::from_utf8_lossy(&out.stdout),
        &String::from_utf8_lossy(&out.stderr),
    );
    Verdict {
        name: c.name.clone(),
        kinds,
        detail,
    }
}

/// Bounds an interpreter cell's address space.
///
/// A release that runs twice can corrupt a stored length, and a corrupt length ends in an
/// unbounded ALLOCATION as often as in a bad read — a time limit does not bound that, and
/// `LOFT_MEMORY_LIMIT` is armed only under `loft test`.  A cell peaks near 22 MiB of address
/// space, so 2 GiB never binds on a working one.  A native cell is left unbounded: its driver
/// runs `rustc`, which would inherit the limit.
#[cfg(target_os = "linux")]
fn cap_address_space(cmd: &mut Command, mode: &str) {
    use std::os::unix::process::CommandExt as _;
    if mode != "--interpret" {
        return;
    }
    // SAFETY: the closure runs in the forked child before `exec` and calls only the
    // async-signal-safe `setrlimit`; it touches no allocator or lock.
    unsafe {
        cmd.pre_exec(|| {
            let limit = libc::rlimit {
                rlim_cur: 2 << 30,
                rlim_max: 2 << 30,
            };
            libc::setrlimit(libc::RLIMIT_AS, &raw const limit);
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn cap_address_space(_cmd: &mut Command, _mode: &str) {}

/// Runs every cell alone, `workers` at a time, and answers the verdicts in cell order.
fn run_all(cells: &[Cell], mode: &str, timeout: &str, workers: usize) -> Vec<Verdict> {
    for_each_cell(cells, mode.trim_start_matches('-'), workers, |dir, c| {
        run_cell(dir, c, mode, timeout)
    })
}

/// Answers `run(dir, cell)` for every cell, `workers` at a time, in cell order.  `dir` is a
/// scratch directory of its own, named by `tag`, removed afterwards.
fn for_each_cell<T: Send>(
    cells: &[Cell],
    tag: &str,
    workers: usize,
    run: impl Fn(&Path, &Cell) -> T + Sync,
) -> Vec<T> {
    let dir = std::env::temp_dir().join(format!("loft_drop_gate_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the gate's scratch directory");
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<T>>> = Mutex::new(cells.iter().map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..workers.max(1) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= cells.len() {
                        break;
                    }
                    let v = run(&dir, &cells[i]);
                    results.lock().unwrap()[i] = Some(v);
                }
            });
        }
    });
    let _ = std::fs::remove_dir_all(&dir);
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|v| v.expect("every cell ran"))
        .collect()
}

fn workers(cap: usize) -> usize {
    std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .min(cap)
}

// ── the baseline ────────────────────────────────────────────────────────────────────────────

fn render(verdicts: &[Verdict], backend: &str) -> String {
    let mut s = format!(
        "# ownership_drop_gate — the {backend} cells that are NOT clean, measured on the tree that\n\
         # committed this file.  Each line is an open (H-Drop) defect: `cell KIND,KIND`.\n\
         # Regenerate only after reading the diff: {BLESS}=1 cargo test --release --test ownership_drop_gate\n"
    );
    let mut lines: Vec<String> = verdicts
        .iter()
        .filter(|v| !v.kinds.is_empty())
        .map(|v| {
            format!(
                "{} {}",
                v.name,
                v.kinds.iter().copied().collect::<Vec<_>>().join(",")
            )
        })
        .collect();
    lines.sort();
    for l in lines {
        s.push_str(&l);
        s.push('\n');
    }
    s
}

fn parse_baseline(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (name, kinds) = l.split_once(' ')?;
            Some((name.to_string(), kinds.trim().to_string()))
        })
        .collect()
}

/// Compares the measured verdicts with the pinned baseline, or writes it under `BLESS`.
///
/// A missing baseline FAILS rather than blessing itself: a gate that writes its own
/// expectation on first sight would pass on any tree, including the one it exists to catch.
fn check_baseline(verdicts: &[Verdict], path: &str, backend: &str) {
    let rendered = render(verdicts, backend);
    if std::env::var_os(BLESS).is_some() {
        std::fs::write(path, &rendered).unwrap_or_else(|e| panic!("write {path}: {e}"));
        eprintln!("ownership_drop_gate: blessed {path}");
        return;
    }
    let Ok(pinned) = std::fs::read_to_string(path) else {
        panic!("{path} is missing — measure it with {BLESS}=1 and READ it before committing");
    };
    let want = parse_baseline(&pinned);
    let got = parse_baseline(&rendered);
    let by_name: HashMap<&str, &Verdict> = verdicts.iter().map(|v| (v.name.as_str(), v)).collect();
    let mut report = String::new();
    for (name, kinds) in &got {
        if want.get(name) != Some(kinds) {
            let was = want.get(name).map_or("clean", String::as_str);
            let _ = writeln!(report, "  NEW   {name}: {was} -> {kinds}");
            for d in &by_name[name.as_str()].detail {
                let _ = writeln!(report, "          {d}");
            }
        }
    }
    for (name, kinds) in &want {
        if !got.contains_key(name) {
            let _ = writeln!(
                report,
                "  GONE  {name}: {kinds} -> clean   (fixed: retire the line with the fix)"
            );
        }
    }
    let clean = verdicts.iter().filter(|v| v.kinds.is_empty()).count();
    eprintln!(
        "ownership_drop_gate ({backend}): {} cells, {clean} clean, {} pinned",
        verdicts.len(),
        got.len()
    );
    assert!(
        report.is_empty(),
        "\n(H-Drop) gate ({backend}) differs from {path}:\n{report}\n\
         A NEW line is a cell that now releases wrongly, or differently — a regression unless \
         the change was meant to move it.  A GONE line is a fix; retire it in the same commit.\n\
         After reading every line: {BLESS}=1 cargo test --release --test ownership_drop_gate\n"
    );
}

// ── the tests ───────────────────────────────────────────────────────────────────────────────

#[test]
fn every_cell_releases_each_resource_once_on_the_interpreter() {
    let cells = all_cells();
    let verdicts = run_all(&cells, "--interpret", "60", workers(16));
    check_baseline(&verdicts, BASELINE, "interpreter");
}

/// The same cells on `--native`, against a baseline of their own: the backends disagree on three
/// cells today, and each disagreement is a finding rather than noise, so it is printed on every
/// run.  One `rustc` per cell measured 36 s for the whole family on a warm target.
#[test]
fn every_cell_releases_each_resource_once_on_native() {
    let cells = all_cells();
    let native = run_all(&cells, "--native", "300", workers(6));
    let interp = run_all(&cells, "--interpret", "60", workers(16));
    for (n, i) in native.iter().zip(&interp) {
        if n.kinds != i.kinds {
            eprintln!(
                "  backends differ  {}: interpreter {:?}, native {:?}",
                n.name, i.kinds, n.kinds
            );
        }
    }
    check_baseline(&native, NATIVE_BASELINE, "native");
}

/// Every cell has its own name, and every generated cell mints through one of the prelude's
/// minting helpers — a cell that could not mint could only ever be clean.
#[test]
fn every_cell_is_distinct_and_mints() {
    let cells = all_cells();
    let names: HashSet<&str> = cells.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names.len(), cells.len(), "two cells share a name");
    for c in &cells {
        assert!(
            ["mk(", "mk_s(", "lit("].iter().any(|m| c.text.contains(m)),
            "{} never mints",
            c.name
        );
    }
}

// ── the copy census (@PLN163 P0) ────────────────────────────────────────────────────────────

/// The `LOFT_DROP_COPY_CENSUS` report for one cell compiled over `prelude`: the site lines of the
/// cell's OWN functions (the prelude's helpers copy too, in every cell) without the scope pass's
/// release snapshots, and the count line.  `None` when the census printed no count line, which
/// means it never ran.
fn census(dir: &Path, c: &Cell, prelude: &str) -> Option<(Vec<String>, String)> {
    let path = dir.join(format!("{}.loft", c.name));
    let text = program(c).replacen(PRELUDE, prelude, 1);
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", c.name));
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .current_dir(dir)
        .env("LOFT_TIMEOUT", "60")
        .env("LOFT_DROP_COPY_CENSUS", "1")
        .output()
        .unwrap_or_else(|e| panic!("spawn loft for {}: {e}", c.name));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let count = stderr
        .lines()
        .find(|l| l.starts_with("drop-copy census: "))?
        .to_string();
    let own = [
        format!("n_{}", c.name),
        format!("n_{}_b", c.name),
        format!("n_{}_m", c.name),
    ];
    let sites = stderr
        .lines()
        .filter(|l| l.starts_with("drop-copy fn=") && !l.contains(" kind=snapshot "))
        .filter(|l| census_field(l, "fn").is_some_and(|f| own.iter().any(|o| o == f)))
        .map(str::to_string)
        .collect();
    Some((sites, count))
}

/// The value of `key=` in a census line.
fn census_field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .find_map(|t| t.strip_prefix(key)?.strip_prefix('='))
}

/// The variables a cell's copy must be reported FROM, derived by hand from its two axes.
///
/// A source that names a structure is reported from that structure's root when the destination
/// COPIES it: a field, an enum payload, an element, a whole value bound to a variable
/// (binding.md `(B-Copy)`).  A PROJECTION bound to a variable or placed in a tuple member is a
/// VIEW of its root instead — `(B-View)`, `(B-View-Depth)`, `(B-View-Base)` — and so is a
/// parameter or a `??` placed in a tuple member, so those copy nothing.  One tuple destination is
/// measured to COPY where the rules predict a view: a tuple MEMBER placed in a tuple, `(tt.0, 1)`,
/// copies while `(s.h, 1)` views (@PLN163 P1 reads that disagreement).  A fresh source (a call, a
/// literal, a call result's field) is reported from no user variable.  `None` for a destination
/// whose copy is not decided yet: a `return` may hand a local over without copying it.
fn expected_roots(name: &str) -> Option<Vec<&'static str>> {
    const VIEWING: &[&str] = &[
        "local", "annot", "nullable", "arm", "arm0", "reassign", "tuplem",
    ];
    let parts: Vec<&str> = name.split('_').collect();
    match parts.as_slice() {
        [_, _, "ret"] | [_, _, _, _, "ret"] => None,
        ["c", source, dest] => {
            let views = VIEWING.contains(dest);
            let tuplem = *dest == "tuplem";
            Some(match *source {
                "local" => vec!["a"],
                "coalesce" | "param" if !tuplem => vec![if *source == "param" { "p" } else { "a" }],
                "tuple" if !views || tuplem => vec!["tt"],
                "field" if !views => vec!["s"],
                "elem" if !views => vec!["vs"],
                "pfield" if !views => vec!["p"],
                _ => vec![],
            })
        }
        ["q", _, _, _, "tuplem"] => Some(vec![]),
        ["q", _, a, b, dest] => {
            let mut roots = Vec::new();
            if *a != "field" || !VIEWING.contains(dest) {
                roots.push(if *a == "field" { "s" } else { "a" });
            }
            if *b == "var" {
                roots.push("b");
            }
            Some(roots)
        }
        _ => None,
    }
}

/// The census lists the copy each generated cell makes: a copy of a structure is reported from
/// that structure's root, a fresh value is reported from no user variable, and a program whose
/// resource type has no `OpDrop` reports no site at all.  The census is what @PLN163 P2 turns
/// into the refusal report, so a copy it cannot see is a copy the refusal would let through.
///
/// It is also the oracle for both verdicts in `src/lease.rs` (@PLN163 P2r).  The `lease=` column —
/// the rule read off the line — refuses a copy in exactly the cells [`lease_verdict`] refuses and
/// in no other.  The `liveness=` column — whether the copied value is used afterwards, kept for
/// `(H-Elide)` — refuses in exactly the cells [`liveness_verdict`] refuses, and never reports a copy
/// its pass did not reach.  Both oracles are derived by hand from each cell's own lines, so neither
/// column can agree with them by construction.  A tuple literal's `item` site is a placement the
/// rule judges, not an emitted copy, so the expected roots below leave it out.
#[test]
fn the_census_names_the_copy_each_cell_makes() {
    let cells = all_cells();
    let reports = for_each_cell(&cells, "census", workers(16), |dir, c| {
        census(dir, c, PRELUDE)
    });
    let mut wrong = Vec::new();
    for (c, report) in cells.iter().zip(reports) {
        let Some((sites, _)) = report else {
            wrong.push(format!("{}: the census never ran", c.name));
            continue;
        };
        // `from=a,b` lists one root per arm of a join; `-` names no variable.
        let from: Vec<&str> = sites
            .iter()
            .filter(|l| census_field(l, "kind") != Some("item"))
            .filter_map(|l| census_field(l, "from"))
            .flat_map(|f| f.split(','))
            .filter(|f| *f != "-")
            .collect();
        let refused_in = |column: &str| -> Vec<&str> {
            sites
                .iter()
                .filter_map(|l| census_field(l, column))
                .filter(|l| l.starts_with("refuse:"))
                .collect()
        };
        let (refusals, liveness_refusals) = (refused_in("lease"), refused_in("liveness"));
        if sites
            .iter()
            .any(|l| census_field(l, "liveness") == Some("unreached"))
        {
            wrong.push(format!(
                "{}: a copy the liveness pass never reached in {sites:?}",
                c.name
            ));
        }
        match lease_verdict(&c.name) {
            Lease::Refused if refusals.is_empty() => wrong.push(format!(
                "{}: refused by the lease rules, but the census refuses nothing in {sites:?}",
                c.name
            )),
            Lease::Once if !refusals.is_empty() => wrong.push(format!(
                "{}: releases once by the lease rules, but the census refuses {refusals:?}",
                c.name
            )),
            _ => {}
        }
        match liveness_verdict(&c.name) {
            Some(Lease::Refused) if liveness_refusals.is_empty() => wrong.push(format!(
                "{}: refused by the liveness reading, but `liveness=` refuses nothing in {sites:?}",
                c.name
            )),
            Some(Lease::Once) if !liveness_refusals.is_empty() => wrong.push(format!(
                "{}: releases once by the liveness reading, but `liveness=` refuses \
                 {liveness_refusals:?}",
                c.name
            )),
            _ => {}
        }
        let Some(roots) = expected_roots(&c.name) else {
            if !c.name.starts_with("p_") {
                eprintln!("  undecided {}: {sites:?}", c.name);
            }
            continue;
        };
        for root in &roots {
            if !from.contains(root) {
                wrong.push(format!("{}: no copy from `{root}` in {sites:?}", c.name));
            }
        }
        for f in &from {
            if !f.starts_with('_') && !roots.contains(f) {
                wrong.push(format!(
                    "{}: a copy from `{f}`, expected only {roots:?}",
                    c.name
                ));
            }
        }
    }

    // The negative half: the same copies of a type with no hook are nobody's to report.
    let plain = PRELUDE.replacen("fn OpDrop(self: H) { println(\"D{self.id}\"); }\n", "", 1);
    assert_ne!(plain, PRELUDE, "the prelude's hook line was not found");
    let locals: Vec<Cell> = cross_cells()
        .into_iter()
        .filter(|c| c.name.starts_with("c_local_"))
        .collect();
    let plain_reports = for_each_cell(&locals, "census_plain", workers(16), |dir, c| {
        census(dir, c, &plain)
    });
    for (c, report) in locals.iter().zip(plain_reports) {
        match report {
            Some((sites, count)) if sites.is_empty() && count == "drop-copy census: 0 sites" => {}
            other => wrong.push(format!("{} without a hook: {other:?}", c.name)),
        }
    }
    assert!(
        wrong.is_empty(),
        "\ncopy census:\n  {}\n",
        wrong.join("\n  ")
    );
}

// ── the lease verdicts (@PLN163 P1) ─────────────────────────────────────────────────────────

/// What `formal/heap.md`'s copy-lease rules require of a cell.  `H` declares `OpDrop` and no
/// `OpCopy`, so every copy of it is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lease {
    /// Compiles and releases each resource once: every `H` the cell places is fresh, moved by a
    /// written position (H-Move), or reached through a view, an argument or a `&`.
    Once,
    /// The cell writes a copy of an existing `H` (H-Copy-Refuse), so it must not compile.
    /// `D-heap-8` until the refusal exists.
    Refused,
}

/// The pilot cells that write no copy of an existing `H`, each classified by hand.
const PILOT_ONCE: &[&str] = &[
    "p_k4", // a block yields the variable it declares
    "p_k5", "p_k6", "p_r1", "p_g2", "p_s8", // fresh values only
    "p_n1", "p_n2", "p_n3", "p_n4", // fresh values written into places, and a removal
    "p_t1", // a member read through a call result, which nothing outlives
];

/// The pilot cells that write a copy of an existing `H`, each classified by hand.
const PILOT_REFUSED: &[&str] = &[
    "p_k1", "p_k2", "p_k3", "p_h1", "p_i1", // `x = a` of a local
    "p_k7", "p_h2", "p_h3", "p_h4", "p_h5", "p_h6", "p_h7", // `x = p` of a parameter
    "p_o1", // `u = e` of a loop variable
    "p_o2", // `u = t` of a tuple
    "p_o3", "p_o4", // `return` of a view, of a member, of a parameter
    "p_o5", "p_e1", "p_e2", // a member placed in a literal or appended
    "p_l1", "p_l2", "p_i2", "p_s6", "p_s7", // a local as an operand of `??`
    "p_j1", "p_j2", "p_j3", "p_j4", "p_s1", "p_s2", "p_s3", "p_s4",
    "p_s5", // a local in a join
    "p_g1", "p_g3", "p_g4", "p_g5", // a member of a call result
];

/// What the lease rules require of the cell `name`, read — as the rule is read — off the cell's
/// own lines: by hand for a pilot, and from its two axes for a generated cell.
///
/// A fresh source (a call, a literal) is placed where it is produced.  A local, a parameter, a
/// `??` over a local and a member of a call result are existing values, so every position copies
/// them — except a `return` of a local, or of a `??` over locals, which (H-Move) moves.  A member
/// of a variable (`s.h`, `vs[0]`, `tt.0`, `p.h`) bound to a variable or chosen by a join arm is a
/// view; placed in a literal, appended or returned it is a copy.
fn lease_verdict(name: &str) -> Lease {
    const VIEWING: &[&str] = &["local", "annot", "nullable", "arm", "arm0", "reassign"];
    let parts: Vec<&str> = name.split('_').collect();
    match parts.as_slice() {
        ["p", ..] if PILOT_ONCE.contains(&name) => Lease::Once,
        ["p", ..] if PILOT_REFUSED.contains(&name) => Lease::Refused,
        ["c", source, dest] => match *source {
            "call" | "literal" => Lease::Once,
            "local" | "coalesce" if *dest == "ret" => Lease::Once,
            "field" | "elem" | "tuple" | "pfield" if VIEWING.contains(dest) => Lease::Once,
            _ => Lease::Refused,
        },
        // `A ?? B`: a local A is copied except when returned; a parameter A always is; a member A
        // is a view in a viewing position, where a local B is still copied.
        ["q", _, "local", _, "ret"] => Lease::Once,
        ["q", _, "field", b, dest] if VIEWING.contains(dest) && *b != "var" => Lease::Once,
        ["q", ..] => Lease::Refused,
        _ => panic!("{name} has no lease verdict: classify it under formal/heap.md § Drop"),
    }
}

/// The verdicts of the SUPERSEDED reading of `(H-Move)`, under which a copy of a value not used
/// afterwards was a move.  P2a's report (`src/lease.rs`) still implements that reading, so this
/// stays as its oracle until @PLN163 P2 is reworked to the written-move rule.  `None` where that
/// reading left a cell undecided.
fn liveness_verdict(name: &str) -> Option<Lease> {
    const VIEWING: &[&str] = &[
        "local", "annot", "nullable", "arm", "arm0", "reassign", "tuplem",
    ];
    const REFUSED_PILOTS: &[&str] = &[
        "p_k7", "p_o1", "p_o3", "p_o4", "p_o5", "p_l1", "p_l2", "p_h2", "p_h3", "p_h4", "p_h5",
        "p_h6", "p_h7", "p_e1", "p_e2", "p_g1", "p_g3", "p_g4", "p_g5", "p_s4", "p_t1",
    ];
    let parts: Vec<&str> = name.split('_').collect();
    match parts.as_slice() {
        ["p", ..] if REFUSED_PILOTS.contains(&name) => Some(Lease::Refused),
        ["p", ..] => Some(Lease::Once),
        ["c", source, dest] => match *source {
            "call" | "literal" | "local" | "coalesce" => Some(Lease::Once),
            "param" | "tuple" if *dest == "tuplem" => None,
            "param" | "callproj" => Some(Lease::Refused),
            _ if VIEWING.contains(dest) => Some(Lease::Once),
            _ => Some(Lease::Refused),
        },
        ["q", _, "param", _, "tuplem"] => None,
        ["q", _, "param", _, _] => Some(Lease::Refused),
        ["q", _, "field", _, dest] if !VIEWING.contains(dest) => Some(Lease::Refused),
        ["q", ..] => Some(Lease::Once),
        _ => None,
    }
}

/// Each OPEN deviation in `formal/heap.md` that a cell with a `Once` verdict still measures, with
/// those cells.  A refused cell compiles today and is `D-heap-8`'s, so it is not listed.
const LEASE_DEVIATIONS: &[(&str, &[&str])] = &[("D-heap-7", &["q_present_local_var_ret"])];

/// Every cell has a lease verdict, and every cell the rules say must release once while a
/// baseline says it does not is carried by exactly one OPEN deviation in `formal/heap.md`.  A fix
/// that retires a baseline line fails here until its cell leaves the deviation's list, and so
/// does a deviation closed in the register while a cell still measures it.
#[test]
fn every_cell_disagreeing_with_the_lease_rules_names_its_open_deviation() {
    let heap = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/doc/claude/formal/heap.md"
    ))
    .expect("read formal/heap.md");
    let mut wrong = Vec::new();
    for dev in LEASE_DEVIATIONS
        .iter()
        .map(|(d, _)| *d)
        .chain(["D-heap-8", "D-heap-9", "D-heap-11"])
    {
        let header = format!("### {dev} — OPEN");
        if !heap.lines().any(|l| l.starts_with(&header)) {
            wrong.push(format!("{dev} is not OPEN in formal/heap.md"));
        }
    }
    let cells = all_cells();
    for (path, backend) in [(BASELINE, "interpreter"), (NATIVE_BASELINE, "native")] {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let measured = parse_baseline(&text);
        let mut refused = 0;
        for c in &cells {
            let listed: Vec<&str> = LEASE_DEVIATIONS
                .iter()
                .filter(|(_, names)| names.contains(&c.name.as_str()))
                .map(|(d, _)| *d)
                .collect();
            let fails = measured.contains_key(&c.name);
            match lease_verdict(&c.name) {
                Lease::Once if fails && listed.len() != 1 => wrong.push(format!(
                    "{backend} {}: releases wrongly under a Once verdict, listed under {listed:?}",
                    c.name
                )),
                Lease::Once if !fails && !listed.is_empty() => wrong.push(format!(
                    "{backend} {}: clean, and still listed under {listed:?} — retire it there",
                    c.name
                )),
                Lease::Refused if !listed.is_empty() => wrong.push(format!(
                    "{backend} {}: a Refused cell, and listed under {listed:?}",
                    c.name
                )),
                Lease::Refused => refused += 1,
                Lease::Once => {}
            }
        }
        eprintln!(
            "  {backend}: {refused} of {} cells are refused by the lease rules (D-heap-8)",
            cells.len()
        );
    }
    assert!(
        wrong.is_empty(),
        "\nlease verdicts:\n  {}\n",
        wrong.join("\n  ")
    );
}

/// Every copy a code generator EMITS for a droppable has a lease verdict from the census
/// (@PLN163 P2b).  The census reads the IR after the scope pass, and a generator mints some
/// copies only when it emits (`copy_manifest.rs`); a copy that reaches no verdict is a copy the
/// refusal would let through.  Each cell is compiled with both instruments on through
/// `--native-emit`, which runs the interpreter's code generation and then the native generator
/// without compiling the result, and `copy_manifest::report` names — once per generator — every
/// emitted copy of a droppable no verdict covers.
#[test]
fn every_emitted_copy_of_a_droppable_has_a_lease_verdict() {
    let cells = all_cells();
    let reports = for_each_cell(&cells, "lease_manifest", workers(16), |dir, c| {
        let path = dir.join(format!("{}.loft", c.name));
        std::fs::write(&path, program(c)).unwrap_or_else(|e| panic!("write {}: {e}", c.name));
        let out = Command::new(loft_bin())
            .arg("--native-emit")
            .arg(dir.join(format!("{}.rs", c.name)))
            .arg(&path)
            .current_dir(dir)
            .env("LOFT_TIMEOUT", "60")
            .env("LOFT_DROP_COPY_CENSUS", "1")
            .env("LOFT_COPY_MANIFEST", "1")
            .output()
            .unwrap_or_else(|e| panic!("spawn loft for {}: {e}", c.name));
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| {
                l.starts_with("lease-manifest:") || l.trim_start().starts_with("lease-unjudged")
            })
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    let mut wrong = Vec::new();
    // Emitted copies per generator: the interpreter reports first, the native generator second.
    let mut emitted = [0usize; 2];
    for (c, lines) in cells.iter().zip(reports) {
        let summaries: Vec<&String> = lines
            .iter()
            .filter(|l| l.starts_with("lease-manifest:"))
            .collect();
        if summaries.len() != 2 {
            wrong.push(format!(
                "{}: expected a manifest report from both generators, got {summaries:?}",
                c.name
            ));
            continue;
        }
        for (count, summary) in emitted.iter_mut().zip(summaries) {
            *count += summary
                .split_whitespace()
                .nth(1)
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0);
        }
        for l in lines
            .iter()
            .filter(|l| l.trim_start().starts_with("lease-unjudged"))
        {
            wrong.push(format!("{}: {}", c.name, l.trim()));
        }
    }
    assert!(
        emitted.iter().all(|&n| n > 0),
        "a generator emitted no copy of a droppable over all cells, so its half measured nothing: \
         {emitted:?}"
    );
    assert!(
        wrong.is_empty(),
        "\nlease manifest ({emitted:?} emitted copies, interpreter and native):\n  {}\n",
        wrong.join("\n  ")
    );
}

/// The scorer can FAIL: each kind of finding is produced by the trace that should produce it,
/// and only that one.  Without this, a scorer that answered "clean" for everything would pass
/// every baseline it wrote.
#[test]
fn the_scorer_names_each_kind_of_wrong_release() {
    let kinds = |trace: &str, stderr: &str| -> Vec<&'static str> {
        score(&trace.replace(' ', "\n"), stderr)
            .0
            .into_iter()
            .collect()
    };
    let none: Vec<&str> = Vec::new();
    assert_eq!(kinds("Cx M1 R1 D1 Cend", ""), none, "clean");
    assert_eq!(
        kinds("Cx M1 X1 R1 Cend", ""),
        none,
        "an (H-Drop-Not) release is clean at zero"
    );
    assert_eq!(kinds("Cx M1 R1 D1 D1 Cend", ""), ["DOUBLE"]);
    assert_eq!(kinds("Cx M1 R1 Cend", ""), ["LOST"]);
    assert_eq!(kinds("Cx M1 R1 D1 D9 Cend", ""), ["UNMINTED"]);
    assert_eq!(kinds("Cx M1 D1 R1 Cend", ""), ["EARLY"]);
    assert_eq!(kinds("Cx M1 R1 Cend D1", ""), ["LATE"]);
    assert_eq!(kinds("Cx M1 X1 D1 Cend", ""), ["RELEASED_NOT"]);
    // One release of a twice-minted id is not a double: the collision is the whole finding.
    assert_eq!(kinds("Cx M1 M1 R1 D1 Cend", ""), ["REMINT"]);
    assert_eq!(kinds("Cx Cend", ""), ["EMPTY"]);
    assert_eq!(kinds("Cx M1", "thread 'main' panicked"), ["CRASHED"]);
    assert_eq!(kinds("", "error: nope"), ["REFUSED"]);
    assert_eq!(
        kinds("Cx noise M1 hello R1 D1 Cend", ""),
        none,
        "a line that is not an event is ignored"
    );
}
