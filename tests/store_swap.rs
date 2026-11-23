// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN203 rust-writer: counts the Rust writer's store exchanges — its `--native` runs as `--native-rust` until then.
//! `@FR-H-SwapIn` — the rebind copy of a minted result into a reset root is an exchange of the
//! two stores, PINNED on the cells (`tests/scripts/a-rebind-from-a-fresh-result-exchanges-the-
//! stores.loft`) by counting the exchanges `LOFT_TRACE_STORE_SWAP=1` names: the cells' values
//! pass on either form, so only this count sees the rule stop firing — on either backend, and
//! not at all under the switch.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-rebind-from-a-fresh-result-exchanges-the-stores.loft";

/// The exchanges one run of the cells makes, and its output.
fn swaps(mode: &str, env: &[(&str, &str)]) -> (usize, String) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg(mode)
        .arg(&src)
        .env("LOFT_TIMEOUT", "240")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TRACE_STORE_SWAP", "1")
        // `@FR-R-RebindBuffer` takes s12's rebind loop on `--native` by adopting instead of
        // exchanging, and `@FR-O-Move`'s rebind adopt (`bind_lowering::re_bind`) takes it on the
        // interpreter; this pins the exchange itself, so it is taken with both rules off.
        .env("LOFT_NO_REBIND_BUFFER", "1")
        .env("LOFT_NO_REBIND_ADOPTS_MINTED", "1")
        .env_remove("LOFT_NO_STORE_SWAP");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    (
        stderr
            .lines()
            .filter(|l| l.starts_with("[swap-in]"))
            .count(),
        stdout,
    )
}

/// s12's 100 000 rebinds are the floor: each is one exchange on both backends.
#[test]
fn the_rebinds_exchange_their_stores_on_both_backends() {
    // The C backend makes and frees the same stores without counting an exchange, so the
    // census is the interpreter's.
    for mode in ["--interpret"] {
        let (n, out) = swaps(mode, &[]);
        assert!(
            out.contains("done"),
            "{mode}: the cells did not finish:\n{out}"
        );
        assert!(
            n >= 100_000,
            "{mode}: {n} exchanges — the rule stopped firing"
        );
    }
}

#[test]
fn the_switch_keeps_every_copy() {
    for mode in ["--interpret"] {
        let (n, out) = swaps(mode, &[("LOFT_NO_STORE_SWAP", "1")]);
        assert!(
            out.contains("done"),
            "{mode}: the cells did not finish:\n{out}"
        );
        assert_eq!(n, 0, "{mode}: LOFT_NO_STORE_SWAP=1 must keep every copy");
    }
}

/// The three rebinds `c = mk(3, …)`, from a callee that mints its hash, each exchange the
/// stores; `WALK` makes them inside a walk of a hash (`for e in w`, a live scratch) and `AFTER`
/// right after it.  The hash's last version is `mk(3, 20)`: 20 + 21 + 22 = 63, three entries.
const GATE: &str = "struct E { k: integer, v: integer }
fn mk(n: integer, base: integer) -> hash<E[k]> {
  h: hash<E[k]> = [];
  for i in 0..n { h += [E { k: i, v: base + i }]; }
  h
}
fn sumh(h: hash<E[k]>) -> integer { t = 0; for e in h { t += e.v; } t }
fn main() {
  c = mk(1, 0);
  w = mk(3, 0);
  REBINDS
  println(\"sum {sumh(c)} len {len(c)}\");
}
";

/// The exchanges one run of `GATE` with `rebinds` makes, and its output.
fn gate_swaps(mode: &str, tag: &str, rebinds: &str) -> (usize, String) {
    let dir = std::env::temp_dir().join(format!("loft_swap_gate_{}", std::process::id()));
    loft::file_access::create_dir_all(&dir).expect("probe dir");
    let src = dir.join(format!("{tag}_{}.loft", &mode[2..]));
    loft::file_access::write(&src, GATE.replace("REBINDS", rebinds)).expect("write probe");
    let out = loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg(mode)
        .arg(&src)
        .env("LOFT_TIMEOUT", "240")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TRACE_STORE_SWAP", "1")
        .env_remove("LOFT_NO_STORE_SWAP")
        .output()
        .expect("spawn loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    (
        stderr
            .lines()
            .filter(|l| l.starts_with("[swap-in]"))
            .count(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// `@FR-H-SwapIn`'s gates (STORE_MODEL D8): no exchange while a walk scratch is live, on both
/// backends.  The same three rebinds exchange right after the walk and copy inside it, and
/// answer the same either way.
///
/// @falsified-at: none — the gate predates the guard, so the receipt is a SABOTAGE (2026-11-21):
///   with the scratch term struck from both gates (`swap_stores_in_as`'s
///   `!self.live_scratches.is_empty()`, `loft_swap_blocked`'s `loft_nscratches != 0`), the walk
///   run exchanges as the after run does and the difference reads 0, not 3, on both backends.
///   CHANNEL: the `[swap-in]` count.  WITNESS: 5 and 5 instead of 5 and 2.
///   HOLDS: `sum 63 len 3` on every run, gate or no gate.
#[test]
fn no_store_is_exchanged_while_a_walk_is_live_on_both_backends() {
    for mode in ["--interpret", "--native"] {
        let (walk, wout) = gate_swaps(mode, "walk", "for e in w { c = mk(3, e.k * 10); }");
        let (after, aout) = gate_swaps(
            mode,
            "after",
            "for e in w { t = e.k; }\n  for r in 0..3 { c = mk(3, r * 10); }",
        );
        assert!(wout.contains("sum 63 len 3"), "{mode} walk: {wout}");
        assert!(aout.contains("sum 63 len 3"), "{mode} after: {aout}");
        assert_eq!(
            after - walk,
            3,
            "{mode}: the three rebinds exchange after the walk ({after}) and copy inside it ({walk})"
        );
    }
}

/// A root record holding `COLL<Item[k]>`, rebound in a loop from a call that answers one of
/// its arguments (copied into the call's buffer, then into `x`).  Per iteration on `q` (three
/// items): 10 + 5 + 6 + 77 + 3 + 3 = 104; on `p` (two): 10 + 5 + 4 + 77 + 3 + 2 = 101; three
/// of each is 615.  `p` and `q` keep their 2 and 3 items; the last pick is `p`, `inv2`.
const KEYED: &str = "struct Item { k: integer, name: text }
struct Inv { title: text, stock: COLL<Item[k]> }
fn load(n: integer) -> Inv {
  r = Inv { title: \"inv{n}\", stock: [] };
  for i in 0..n { r.stock += [Item { k: i * 10, name: \"i{i}\" }]; }
  r
}
fn pick(a: Inv, b: Inv, i: integer) -> Inv { if i % 2 == 1 { a } else { b } }
fn main() {
  p = load(2);
  q = load(3);
  x = load(1);
  total = 0;
  for r in 0..6 {
    x = pick(p, q, r);
    total += if x.stock[10] { x.stock[10].k } else { -1 };
    total += if x.stock[999] { 1 } else { 5 };
    for it in x.stock { total += len(it.name); }
    x.stock += [Item { k: 77, name: \"new\" }];
    total += if x.stock[77] { x.stock[77].k } else { -1 };
    x.stock[0] = null;
    total += (if x.stock[0] { 1 } else { 3 }) + len(x.stock);
  }
  println(\"{total} {len(p.stock)} {len(q.stock)} {x.title}\");
}
";

/// The same rebind of a record holding child records (`Node?` fields of a non-null root).
/// Per iteration 111n + 2, then 7 + 5 after a child is replaced: 236 on `q` (n = 2), 125 on
/// `p` (n = 1); three of each is 1083.  `p` and `q` keep their left keys 10 and 20.
const CHILDREN: &str = "struct Node { key: integer, name: text, left: Node?, right: Node? }
struct Holder { title: text, root: Node }
fn grow(n: integer) -> Holder {
  h = Holder { title: \"h{n}\", root: Node { key: n, name: \"r{n}\" } };
  h.root.left = Node { key: n * 10, name: \"L\" };
  h.root.right = Node { key: n * 100, name: \"RR\" };
  h
}
fn pick(a: Holder, b: Holder, i: integer) -> Holder { if i % 2 == 1 { a } else { b } }
fn main() {
  p = grow(1);
  q = grow(2);
  x = grow(3);
  total = 0;
  for r in 0..6 {
    x = pick(p, q, r);
    total += x.root.key + x.root.left.key + x.root.right.key + len(x.root.right.name);
    x.root.left = Node { key: 7, name: \"seven\" };
    total += x.root.left.key + len(x.root.left.name);
  }
  println(\"{total} {p.root.left.key} {q.root.left.key} {x.title}\");
}
";

/// The same rebind of a record holding a `reference<T>`, which names a slot and so keeps the
/// copy: on `q`, fav 6 + 2 + 2 items = 10; on `p`, 2 + 2 + 2 = 6; three of each is 48.
const REFERENCE: &str = "struct Item { k: integer, name: text }
struct Holder { title: text, items: vector<Item>, fav: reference<Item>? }
fn mk(n: integer) -> Holder {
  h = Holder { title: \"h{n}\", items: [Item { k: n, name: \"a\" }, Item { k: n + 1, name: \"bb\" }] };
  h.fav = h.items[1];
  h
}
fn pick(a: Holder, b: Holder, i: integer) -> Holder { if i % 2 == 1 { a } else { b } }
fn main() {
  p = mk(1);
  q = mk(5);
  x = mk(9);
  total = 0;
  for r in 0..6 {
    x = pick(p, q, r);
    total += x.fav.k + len(x.fav.name) + len(x.items);
  }
  println(\"{total} {p.fav.k} {q.fav.k} {x.title}\");
}
";

/// The exchanges one run of `program` makes under `LOFT_STRICT_STORES` (a stale slot number
/// faults rather than reading another store), and its output.
fn strict_swaps(mode: &str, tag: &str, program: &str) -> (usize, String) {
    let dir = std::env::temp_dir().join(format!("loft_swap_whole_{}", std::process::id()));
    loft::file_access::create_dir_all(&dir).expect("probe dir");
    let src = dir.join(format!("{tag}_{}.loft", &mode[2..]));
    loft::file_access::write(&src, program).expect("write probe");
    let out = loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg(mode)
        .arg(&src)
        .env("LOFT_TIMEOUT", "240")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_STRICT_STORES", "1")
        .env("LOFT_TRACE_STORE_SWAP", "1")
        .env_remove("LOFT_NO_STORE_SWAP")
        .output()
        .expect("spawn loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    (
        stderr
            .lines()
            .filter(|l| l.starts_with("[swap-in]"))
            .count(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// `@FR-H-SwapIn` (STORE_MODEL D39): a root record whose keyed collections and child records
/// keep their pointers as record numbers inside its store moves whole — exchanged on both
/// backends, once per rebind — and one holding a `reference<T>` keeps the copy on both.
///
/// @falsified-at: none — the C always exchanged here, so the receipt is a SABOTAGE
///   (2026-11-21): with the interpreter's walk the old one again (`swap_stores_in_as` asking
///   `tree_holds_no_stored_refs`, which declines every keyed part and child record), the
///   interpreter copies where the C exchanges.
///   CHANNEL: the `[swap-in]` count.  WITNESS: 0 on `--interpret` against 6 on `--native`.
///   HOLDS: every cell's answer, exchange or copy.
#[test]
fn a_record_holding_keyed_collections_or_children_moves_whole_on_both_backends() {
    let cells: Vec<(String, String, &str, usize)> = vec![
        (
            "hash".into(),
            KEYED.replace("COLL", "hash"),
            "615 2 3 inv2",
            6,
        ),
        (
            "sorted".into(),
            KEYED.replace("COLL", "sorted"),
            "615 2 3 inv2",
            6,
        ),
        (
            "index".into(),
            KEYED.replace("COLL", "index"),
            "615 2 3 inv2",
            6,
        ),
        ("children".into(), CHILDREN.to_string(), "1083 10 20 h1", 6),
        ("reference".into(), REFERENCE.to_string(), "48 2 6 h1", 0),
    ];
    for (tag, program, want, swaps) in &cells {
        for mode in ["--interpret", "--native"] {
            let (n, out) = strict_swaps(mode, tag, program);
            assert_eq!(out.trim(), *want, "{tag} {mode}: the answer");
            assert_eq!(n, *swaps, "{tag} {mode}: {n} exchanges, {swaps} expected");
        }
    }
}

/// A local bound from a call and then returned is the caller's buffer, and holds no store when
/// the caller hands a null one: `mk_loc`'s answer is exchanged into a fresh store, reset, on
/// both backends (`@FR-H-SwapIn`) rather than copied into it.  `via(4)` is 4 + 100 with the
/// two points `mk_loc` pushed, the first 4.
const RETURNED: &str = "struct P { pts: vector<float>, n: integer = 0 }
fn mk_loc(i: integer) -> P { o = P { pts: [], n: i }; o.pts += [i as float, 0.25]; o }
fn via(i: integer) -> P { n = mk_loc(i); n.n += 100; n }
fn main() { r = via(4); println(\"{r.n} {len(r.pts)} {r.pts[0]?}\"); }
";

/// @falsified-at: none — the interpreter always exchanged here, so the receipt is a SABOTAGE
///   (2026-11-21): with `loft_swap_rebind` declining a destination that holds no store again
///   (`src/c/loft_rt.h`), the C copies `mk_loc`'s answer into the store it makes.
///   CHANNEL: the `[swap-in]` count.  WITNESS: 0 on `--native` against 1.
///   HOLDS: `104 2 4` either way.
#[test]
fn a_returned_local_bound_from_a_call_takes_the_answer_whole_on_both_backends() {
    for mode in ["--interpret", "--native"] {
        let (n, out) = strict_swaps(mode, "returned", RETURNED);
        assert_eq!(out.trim(), "104 2 4", "{mode}: the answer");
        assert_eq!(n, 1, "{mode}: {n} exchanges, 1 expected");
    }
}

/// A rebind from a call whose answer is copied (R2b) and given up, into a local that holds a
/// literal: the answer is exchanged into the local's own store on both backends (STORE_MODEL
/// D43).  `either` answers `w` (i = 0) then `u` (i = 1): 1 + 8 + 1 + 6 = 16, and `u`, `w` keep
/// 5 and 7.  The interpreter makes six stores: `u`, `w`, and per iteration the literal and the
/// call's buffer.  The copy into a fresh store it took before made one more per iteration.
const ANSWER_COPIED: &str = "struct S { v: integer, t: text }
fn either(a: S, b: S, i: integer) -> S { if i % 2 == 1 { a } else { b } }
fn one_of_two(u: S, w: S) -> integer {
  tot = 0;
  for i in 0..2 {
    x = S { v: 1, t: \"x{i}\" };
    tot += x.v;
    x = either(u, w, i);
    x.v += 1;
    tot += x.v;
  }
  tot
}
fn main() { u = S { v: 5, t: \"u\" }; w = S { v: 7, t: \"w\" }; println(\"{one_of_two(u, w)} {u.v} {w.v}\"); }
";

/// The interpreter's store makes for `ANSWER_COPIED` (`LOFT_TRACE_DB`'s `OpDatabase` lines)
/// under `env`, and its output.
fn interpreter_makes(env: &[(&str, &str)]) -> (usize, String) {
    let dir = std::env::temp_dir().join(format!("loft_swap_copied_{}", std::process::id()));
    loft::file_access::create_dir_all(&dir).expect("probe dir");
    let src = dir.join("copied.loft");
    loft::file_access::write(&src, ANSWER_COPIED).expect("write probe");
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--interpret")
        .arg(&src)
        .env("LOFT_TRACE_DB", "1")
        .env("LOFT_STRICT_STORES", "1")
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    (
        stderr.lines().filter(|l| l.contains("OpDatabase")).count(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// @falsified-at: none — the receipt is the switch the test runs itself:
///   `LOFT_NO_SWAP_REBIND=1` frees the local first and copies into a fresh store, 8 makes.
///   CHANNEL: `LOFT_TRACE_DB`'s make count.  WITNESS: 8 against 6.  HOLDS: `16 5 7`.
#[test]
fn a_copied_answer_is_exchanged_into_the_store_the_local_holds() {
    let (on, out) = interpreter_makes(&[]);
    assert_eq!(out.trim(), "16 5 7", "--interpret: the answer");
    assert_eq!(on, 6, "--interpret: the stores made with the exchange");
    let (off, out) = interpreter_makes(&[("LOFT_NO_SWAP_REBIND", "1")]);
    assert_eq!(
        out.trim(),
        "16 5 7",
        "--interpret under the switch: the answer"
    );
    assert_eq!(off, 8, "--interpret: the stores made without the exchange");
    let (n, out) = strict_swaps("--native", "copied", ANSWER_COPIED);
    assert_eq!(out.trim(), "16 5 7", "--native: the answer");
    assert_eq!(n, 2, "--native: one exchange per iteration");
}
