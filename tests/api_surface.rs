// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 C1 commit 1 — `loft api-surface`: the observable public surface as membership
//! and visibility TIER. Proves the closure walk (a non-`pub` type reachable through a `pub`
//! signature is SEALED, not dropped) and the exclusions (a private / unreachable non-`pub`
//! type and a non-`pub` fn are not in the surface).

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static SEQ: AtomicU32 = AtomicU32::new(0);

fn api_surface(src: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "loft_apisurf_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.loft");
    std::fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("api-surface")
        .arg(&file)
        .output()
        .expect("run loft api-surface");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "api-surface exited non-zero: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Run `loft api-surface --diff <base> <new> [--json]`; return (stdout, exit code).
fn api_diff_cli(base: &str, new: &str, json: bool) -> (String, i32) {
    let dir = std::env::temp_dir().join(format!(
        "loft_apidiff_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let fb = dir.join("base.loft");
    let fn_ = dir.join("new.loft");
    std::fs::write(&fb, base).unwrap();
    std::fs::write(&fn_, new).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("api-surface").arg("--diff").arg(&fb).arg(&fn_);
    if json {
        cmd.arg("--json");
    }
    let out = cmd.output().expect("run loft api-surface --diff");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// C123 — `both` is a second SPELLING of `self`, not a second member.
///
/// A `both` receiver registers a bare-name `Dynamic` dispatcher targeting its own method. That
/// alias is not observable to a caller (C123 measured the two spellings identical, import lists
/// included), so recording it made the migration C123 prescribes — it names `regex`'s `matches` —
/// diff as `removed fn`, refusing the very rename the deprecation asks for.
#[test]
fn a_both_receiver_records_no_bare_name_phantom() {
    let out = api_surface("pub fn f(both: text, p: text) -> boolean { len(both) > len(p) }\n");
    assert!(
        out.lines().any(|l| l.starts_with("text.f · method")),
        "the method itself is still recorded:\n{out}"
    );
    assert!(
        !out.lines().any(|l| l.starts_with("f · fn")),
        "the bare-name alias must NOT be a member of its own:\n{out}"
    );
}

/// The control for the rule above, and the reason it is not a blanket skip of `Dynamic`: for a
/// real OVERLOAD SET the dispatcher is the only member carrying the name a consumer writes — the
/// overloads themselves live under mangled keys (`f_15integer#integer_pick`) nobody can call.
#[test]
fn an_overload_set_keeps_its_dispatcher_member() {
    let out = api_surface(
        "pub fn pick(a: integer, b: integer) -> integer { a }\n\
         pub fn pick(s: text, b: integer) -> text { s }\n",
    );
    assert!(
        out.lines().any(|l| l.starts_with("pick · fn")),
        "an overload set's dispatcher IS its callable name:\n{out}"
    );
}

/// End to end: the rename C123 prescribes must diff as a superset, not a break.
#[test]
fn renaming_a_both_receiver_to_self_is_not_a_break() {
    let (out, _code) = api_diff_cli(
        "pub fn f(both: text, p: text) -> boolean { len(both) > len(p) }\n",
        "pub fn f(self: text, p: text) -> boolean { len(self) > len(p) }\n",
        true,
    );
    assert!(
        out.contains(r#""api":{"verdict":"superset""#),
        "both->self is the sanctioned migration, not a break:\n{out}"
    );
}

/// Commit 7 — the PR-check round-trip: `--emit-baseline` on `released`, then `--check` the
/// baseline against `current`. Returns (stdout, exit code).
fn emit_and_check(released: &str, current: &str) -> (String, i32) {
    let dir = std::env::temp_dir().join(format!(
        "loft_apibase_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let released_f = dir.join("released.loft");
    let baseline_f = dir.join("lib.api-baseline");
    let current_f = dir.join("current.loft");
    std::fs::write(&released_f, released).unwrap();
    std::fs::write(&current_f, current).unwrap();
    let emit = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("api-surface")
        .arg(&released_f)
        .arg("--emit-baseline")
        .output()
        .expect("emit-baseline");
    assert!(
        emit.status.success(),
        "emit-baseline failed: {}",
        String::from_utf8_lossy(&emit.stderr)
    );
    std::fs::write(&baseline_f, &emit.stdout).unwrap();
    let chk = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("api-surface")
        .arg("--check")
        .arg(&baseline_f)
        .arg(&current_f)
        .output()
        .expect("check");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&chk.stdout).into_owned(),
        chk.status.code().unwrap_or(-1),
    )
}

#[test]
fn surface_membership_and_tiers() {
    let s = api_surface(
        "struct Widget { x: integer }\n\
         struct Hidden { z: integer }\n\
         pub struct Public { v: integer }\n\
         pub fn make() -> Widget { Widget { x: 5 } }\n\
         pub fn plain(a: integer) -> integer { a + 1 }\n\
         fn helper() -> Hidden { Hidden { z: 0 } }\n",
    );
    // public roots
    assert!(s.contains("make · fn · public"), "make missing:\n{s}");
    assert!(s.contains("plain · fn · public"), "plain missing:\n{s}");
    assert!(
        s.contains("Public · struct · public"),
        "Public missing:\n{s}"
    );
    // the closure: a non-`pub` type returned by a `pub` fn is SEALED, not dropped.
    assert!(
        s.contains("Widget · struct · sealed"),
        "Widget not sealed:\n{s}"
    );
    // exclusions: a private/unreachable non-`pub` type and a non-`pub` fn are NOT surface.
    assert!(!s.contains("Hidden"), "Hidden must be excluded:\n{s}");
    assert!(!s.contains("helper"), "helper must be excluded:\n{s}");
}

#[test]
fn closure_is_transitive() {
    // `build` returns `Outer` (sealed); `Outer` has a field of non-`pub` `Inner` → `Inner`
    // is sealed too. Proves the closure follows struct field types, transitively.
    let s = api_surface(
        "struct Inner { n: integer }\n\
         struct Outer { i: Inner }\n\
         pub fn build() -> Outer { Outer { i: Inner { n: 1 } } }\n",
    );
    assert!(s.contains("build · fn · public"), "build missing:\n{s}");
    assert!(
        s.contains("Outer · struct · sealed"),
        "Outer not sealed:\n{s}"
    );
    assert!(
        s.contains("Inner · struct · sealed"),
        "Inner not sealed (transitive):\n{s}"
    );
}

#[test]
fn signatures_over_every_kind() {
    // Commit 2 — resolved signatures attached, in the clean user-facing type spelling.
    let s = api_surface(
        "struct Widget { x: integer, tag: text }\n\
         enum Shape { Circle { r: integer }, Square { side: integer }, Point }\n\
         pub struct Public { v: integer }\n\
         pub fn make(n: integer, label: text) -> Widget { Widget { x: n, tag: label } }\n\
         pub fn maybe(a: integer) -> Widget? { if a > 0 { Widget { x: a, tag: \"\" } } else { null } }\n\
         pub fn area(s: Shape) -> integer { 0 }\n",
    );
    let has = |line: &str| s.lines().any(|l| l == line);
    // fn: params (name: type) + return; a nullable return renders as `?`.
    assert!(
        has("make · fn · public · (n: integer, label: text) -> Widget"),
        "make sig:\n{s}"
    );
    assert!(
        has("maybe · fn · public · (a: integer) -> Widget?"),
        "nullable return:\n{s}"
    );
    assert!(
        has("area · fn · public · (s: Shape) -> integer"),
        "area sig:\n{s}"
    );
    // struct fields — a public root and a sealed closure member. Fields are sorted by name
    // (commit 3 canonicalisation: named construction → field order is not API), so `tag`
    // precedes `x` regardless of declaration order.
    assert!(
        has("Public · struct · public · { v: integer }"),
        "Public sig:\n{s}"
    );
    assert!(
        has("Widget · struct · sealed · { tag: text, x: integer }"),
        "Widget sig:\n{s}"
    );
    // enum variants, sorted by name, with the synthetic `enum` discriminant tag filtered out.
    assert!(
        has("Shape · enum · sealed · { Circle { r: integer }, Point, Square { side: integer } }"),
        "enum sig:\n{s}"
    );
}

#[test]
fn determinism_corpus() {
    // Commit 3 — the make-or-break. Cosmetically-different-but-identical surfaces MUST
    // produce byte-identical descriptors (a strict check has no escape valve for a false
    // break); a genuinely-different surface MUST differ.
    let same = |a: &str, b: &str, why: &str| {
        assert_eq!(api_surface(a), api_surface(b), "must be identical: {why}");
    };
    let differ = |a: &str, b: &str, why: &str| {
        assert_ne!(api_surface(a), api_surface(b), "must differ: {why}");
    };

    // --- invariances: a cosmetic edit is NOT a diff ---
    same(
        "pub fn f() -> integer { 1 }\nstruct S { a: integer }\npub fn g() -> S { S{a:1} }\n",
        "struct S { a: integer }\npub fn g() -> S { S{a:1} }\npub fn f() -> integer { 1 }\n",
        "reordered top-level defs",
    );
    same(
        "pub struct W { x: integer, tag: text }\n",
        "pub struct W { tag: text, x: integer }\n",
        "reordered struct fields (named construction → not API)",
    );
    same(
        "pub enum E { A { p: integer, q: text }, B }\n",
        "pub enum E { B, A { q: text, p: integer } }\n",
        "reordered enum variants + variant fields",
    );
    same(
        "pub struct W{x:integer}\n",
        "pub struct W {  x : integer  }\n",
        "whitespace / formatting",
    );
    same(
        "type Score = integer;\npub fn f() -> Score { 1 }\n",
        "pub fn f() -> integer { 1 }\n",
        "a transparent alias vs its expansion",
    );

    // --- real changes MUST differ (positive controls — no vacuous determinism) ---
    differ(
        "pub fn f() -> integer { 1 }\n",
        "pub fn f() -> text { \"\" }\n",
        "return type change",
    );
    differ(
        "pub fn f(a: integer, b: text) -> integer { a }\n",
        "pub fn f(b: text, a: integer) -> integer { a }\n",
        "fn param REORDER (positional — a real API change, must NOT be canonicalised away)",
    );
    differ(
        "pub struct W { x: integer }\n",
        "pub struct W { x: text }\n",
        "field type change",
    );
    differ(
        "pub struct W { x: integer }\n",
        "pub struct W { x: integer, y: text }\n",
        "added field",
    );
}

#[test]
fn diff_cli_verdict_and_exit_codes() {
    // Commit 6 — `--diff` wires the surface reader to the diff engine.
    // Superset (added a fn) → exit 0, "drop-in".
    let (out, code) = api_diff_cli(
        "pub fn make() -> integer { 1 }\n",
        "pub fn make() -> integer { 1 }\npub fn extra(a: integer) -> integer { a }\n",
        false,
    );
    assert_eq!(code, 0, "superset exits 0:\n{out}");
    assert!(out.contains("drop-in"), "superset human text:\n{out}");
    // Break (changed return type) → exit 1, names the broken symbol.
    let (out, code) = api_diff_cli(
        "pub fn make() -> integer { 1 }\n",
        "pub fn make() -> text { \"\" }\n",
        false,
    );
    assert_eq!(code, 1, "break exits 1:\n{out}");
    assert!(
        out.contains("BREAK") && out.contains("make"),
        "break human names make:\n{out}"
    );
}

#[test]
fn diff_cli_json() {
    let (out, code) = api_diff_cli(
        "pub fn make() -> integer { 1 }\n",
        "pub fn make() -> text { \"\" }\n",
        true,
    );
    assert_eq!(code, 1);
    assert!(
        out.contains(r#""verdict":"break""#),
        "json break verdict:\n{out}"
    );
    assert!(out.contains("make"), "json names make:\n{out}");
    let (out, code) = api_diff_cli(
        "pub fn make() -> integer { 1 }\n",
        "pub fn make() -> integer { 1 }\npub fn g() -> integer { 2 }\n",
        true,
    );
    assert_eq!(code, 0);
    assert!(
        out.contains(r#""verdict":"superset""#),
        "json superset verdict:\n{out}"
    );
}

// Commit 5 — the @PLN97 LAYOUT axis: a second verdict beside the API axis.
const POINT_V1: &str = "pub struct Point { x: integer, y: integer }\n\
                        pub fn make() -> Point { Point{x:1,y:2} }\n";
const POINT_REORDERED: &str = "pub struct Point { y: integer, x: integer }\n\
                               pub fn make() -> Point { Point{x:1,y:2} }\n";

#[test]
fn layout_axis_field_reorder_is_api_dropin_but_layout_changed() {
    // A field REORDER is a named-construction API drop-in (commit 3 sorts fields), but a store
    // LAYOUT change — the silent DATA break for a persisting consumer that the API axis alone
    // green-lights. It must red (exit 1) on the layout axis and name the reshaped type.
    let (out, code) = api_diff_cli(POINT_V1, POINT_REORDERED, false);
    assert!(out.contains("API: drop-in"), "API drop-in:\n{out}");
    assert!(
        out.contains("Layout: CHANGED") && out.contains("Point"),
        "layout changed names Point:\n{out}"
    );
    assert_eq!(code, 1, "a layout reshape reds:\n{out}");
}

#[test]
fn layout_axis_stable_on_pure_addition() {
    let added = format!("{POINT_V1}pub fn g() -> integer {{ 0 }}\n");
    let (out, code) = api_diff_cli(POINT_V1, &added, false);
    assert!(out.contains("Layout: stable"), "layout stable:\n{out}");
    assert_eq!(code, 0, "both axes clean → exit 0:\n{out}");
}

#[test]
fn diff_json_carries_both_axes() {
    let (out, _) = api_diff_cli(POINT_V1, POINT_REORDERED, true);
    assert!(
        out.contains(r#""api":{"verdict":"superset""#),
        "json api superset:\n{out}"
    );
    assert!(
        out.contains(r#""layout":{"verdict":"changed""#) && out.contains("Point"),
        "json layout changed names Point:\n{out}"
    );
}

#[test]
fn pr_check_baseline_round_trip() {
    // Commit 7 — the deliverable. Emit a baseline of the released source, then check current
    // against it: a drop-in stays green (exit 0); an injected API break OR layout reshape reds
    // (exit 1) — the positive control per axis, no vacuous green.
    let released = "pub struct Point { x: integer, y: integer }\npub fn make(a: integer) -> Point { Point{x:a,y:0} }\n";

    // drop-in: add a fn
    let (out, code) = emit_and_check(
        released,
        &format!("{released}pub fn extra() -> integer {{ 0 }}\n"),
    );
    assert_eq!(code, 0, "drop-in exits 0:\n{out}");
    assert!(
        out.contains("drop-in") && out.contains("Layout: stable"),
        "drop-in clean on both axes:\n{out}"
    );

    // injected API break: drop a param
    let (out, code) = emit_and_check(
        released,
        "pub struct Point { x: integer, y: integer }\npub fn make() -> Point { Point{x:0,y:0} }\n",
    );
    assert_eq!(code, 1, "an API break reds:\n{out}");
    assert!(
        out.contains("API: BREAK") && out.contains("make"),
        "names make:\n{out}"
    );

    // injected layout reshape: reorder fields (an API drop-in but a data break)
    let (out, code) = emit_and_check(
        released,
        "pub struct Point { y: integer, x: integer }\npub fn make(a: integer) -> Point { Point{x:a,y:0} }\n",
    );
    assert_eq!(code, 1, "a layout reshape reds:\n{out}");
    assert!(
        out.contains("API: drop-in") && out.contains("Layout: CHANGED"),
        "API drop-in but layout changed:\n{out}"
    );
}

#[test]
fn committed_dogfood_baseline_is_a_drop_in() {
    // The committed fixture must stay a drop-in against its committed baseline — `make
    // api-compat`'s green case. Catches a `lib.loft` change that forgot to regenerate the
    // baseline, and a loft change that reshapes its layout.
    let root = env!("CARGO_MANIFEST_DIR");
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("api-surface")
        .arg("--check")
        .arg(format!("{root}/tests/fixtures/api_compat/lib.api-baseline"))
        .arg(format!("{root}/tests/fixtures/api_compat/lib.loft"))
        .output()
        .expect("check committed baseline");
    assert_eq!(
        out.status.code(),
        Some(0),
        "committed dogfood baseline is not a drop-in — regenerate it with \
         `loft api-surface tests/fixtures/api_compat/lib.loft --emit-baseline`:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// loft#1191 — a TRAILING parameter that carries a DEFAULT is additive, and the check used
/// to call it a break.
///
/// [COMPATIBILITY.md § Per-surface](../doc/claude/COMPATIBILITY.md) states the rule this
/// asserts: under **Stdlib API**, *"a new optional parameter"* is listed as additive, and the
/// regression column is *"a signature change that breaks existing calls"* — which a default
/// does not. The old behaviour offered one remedy, raising `api_compatible_with`, so a
/// library adopting the idiom loft recommends had to publish a withdrawal it never made.
///
/// The boundary is the whole content of the rule, so each side of it is asserted here: the
/// old parameter list must remain a PREFIX of the new one and every added parameter must
/// carry a default. Anything else still breaks — an appended required parameter, a default
/// inserted before an existing parameter (which re-binds every positional call), a removed
/// one, and a changed type.
#[test]
fn a_trailing_defaulted_parameter_is_additive_and_nothing_else_is() {
    let base = "pub fn f(a: integer, b: float) -> integer { a }\n";

    // The rule itself.
    let (out, code) = api_diff_cli(
        base,
        "pub fn f(a: integer, b: float, c: boolean = false) -> integer { a }\n",
        false,
    );
    assert_eq!(
        code, 0,
        "a trailing defaulted parameter is a drop-in:\n{out}"
    );
    assert!(out.contains("drop-in"), "human text:\n{out}");

    // Two of them, because "one" is not a rule.
    let (out, code) = api_diff_cli(
        base,
        "pub fn f(a: integer, b: float, c: boolean = false, d: integer = 3) -> integer { a }\n",
        false,
    );
    assert_eq!(code, 0, "two trailing defaults are still a drop-in:\n{out}");

    // A receiver does not change the rule.
    let (out, code) = api_diff_cli(
        "pub struct S { n: integer }\npub fn m(self: S, a: integer) -> integer { a }\n",
        "pub struct S { n: integer }\npub fn m(self: S, a: integer, b: boolean = false) -> integer { a }\n",
        false,
    );
    assert_eq!(code, 0, "a method's trailing default is a drop-in:\n{out}");

    // --- and the four boundaries, each of which must STAY a break ---

    let (out, code) = api_diff_cli(
        base,
        "pub fn f(a: integer, b: float, c: boolean) -> integer { a }\n",
        false,
    );
    assert_eq!(code, 1, "an appended REQUIRED parameter breaks:\n{out}");

    // Inserted before an existing parameter: every positional call re-binds, which is the
    // silent half of the failure and the reason the rule is "trailing", not "defaulted".
    let (out, code) = api_diff_cli(
        base,
        "pub fn f(a: integer, c: boolean = false, b: float) -> integer { a }\n",
        false,
    );
    assert_eq!(code, 1, "a default inserted mid-list breaks:\n{out}");

    let (out, code) = api_diff_cli(
        "pub fn f(a: integer, b: float = 1.0) -> integer { a }\n",
        "pub fn f(a: integer) -> integer { a }\n",
        false,
    );
    assert_eq!(code, 1, "REMOVING a defaulted parameter breaks:\n{out}");

    let (out, code) = api_diff_cli(
        base,
        "pub fn f(a: integer, b: text, c: boolean = false) -> integer { 0 }\n",
        false,
    );
    assert_eq!(
        code, 1,
        "a changed type is a break even beside a legal addition:\n{out}"
    );
}

/// @PLN165 D10 — a library's generic types are listed as their author wrote them: the
/// TEMPLATE (`Grid<T>`, `Slot<T>` as an enum), its methods as methods, and every signature
/// spelling its variables (`T`, never the placeholder key `T#4`).  The compiler's own defs —
/// the variable placeholders, each instance (the template stands for it), and the
/// `main_vector<τ>` wrapper a vector parameter registers — are nobody's surface.
#[test]
fn a_generic_library_lists_its_templates_not_its_instances() {
    let out = api_surface(
        "pub struct Grid<T> { cells: vector<T>, w: integer }\n\
         pub fn grid<T>(cells: vector<T>, w: integer) -> Grid<T> { Grid { cells: cells, w: w } }\n\
         pub fn at<T>(self: Grid<T>, i: integer) -> T? { self.cells[i] }\n\
         pub enum Slot<T> { Full { v: T }, Hole }\n\
         pub fn fulls(v: vector<Slot<integer>>) -> integer { len(v) }\n\
         pub struct Pair<K, V> { k: K, v: V }\n\
         pub fn swap<K, V>(self: Pair<K, V>) -> Pair<V, K> { Pair { k: self.v, v: self.k } }\n",
    );
    assert_eq!(
        out,
        "Grid.at · method · public · (self: Grid<T>, i: integer) -> T?\n\
         Grid<T> · struct · public · { cells: vector<T>, w: integer }\n\
         Pair.swap · method · public · (self: Pair<K, V>) -> Pair<V, K>\n\
         Pair<K, V> · struct · public · { k: K, v: V }\n\
         Slot<T> · enum · public · { Full { v: T }, Hole }\n\
         fulls · fn · public · (v: vector<Slot<integer>>) -> integer\n\
         grid · fn · public · (cells: vector<T>, w: integer) -> Grid<T>\n"
    );
}

/// The wrapper a `vector<τ>` parameter registers is the compiler's, generic or not: it was
/// listed as a public struct of every library with such a parameter, so dropping the last one
/// would have read as removing a type.
#[test]
fn a_vector_parameter_lists_no_wrapper_struct() {
    let out = api_surface(
        "pub struct Mine { n: integer }\npub fn total(v: vector<Mine>) -> integer { len(v) }\n",
    );
    assert!(!out.contains("main_vector"), "{out}");
    assert!(
        out.contains("Mine · struct") && out.contains("total · fn"),
        "{out}"
    );
}

/// A generic library's surface round-trips through its baseline: an unchanged library checks
/// clean, and a second type variable is a break (every `Grid<integer>` a consumer wrote stops
/// naming the type).
#[test]
fn a_generic_librarys_baseline_round_trips() {
    let lib = "pub struct Grid<T> { cells: vector<T>, w: integer }\n\
               pub fn grid<T>(cells: vector<T>, w: integer) -> Grid<T> { Grid { cells: cells, w: w } }\n";
    let (out, code) = emit_and_check(lib, lib);
    assert_eq!(code, 0, "an unchanged generic library checks clean:\n{out}");
    let (out, code) = emit_and_check(
        lib,
        "pub struct Grid<T, U> { cells: vector<T>, w: integer, u: U }\n",
    );
    assert_ne!(code, 0, "a second type variable is a break:\n{out}");
}
