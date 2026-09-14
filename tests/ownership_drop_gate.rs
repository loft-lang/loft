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
];

/// `(name, statement)` over `@E`, the source expression; `ret` returns it from a helper
/// instead, and `arm`/`reassign` mint their other value as `@K`.
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
            let arm = dname == "arm";
            let all_params = join_nonempty(&[params, if arm { "k: integer" } else { "" }]);
            let call_args = join_nonempty(&[cargs, if arm { "1" } else { "" }]);
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
    let dir = std::env::temp_dir().join(format!(
        "loft_drop_gate_{}_{}",
        mode.trim_start_matches('-'),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create the gate's scratch directory");
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Verdict>>> = Mutex::new(cells.iter().map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..workers.max(1) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= cells.len() {
                        break;
                    }
                    let v = run_cell(&dir, &cells[i], mode, timeout);
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
