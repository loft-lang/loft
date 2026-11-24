// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#992 — a droppable local must be released ONCE, after the arm that runs.
//!
//! A `match` in a function's tail position with a `return` in any arm typed the block
//! `never`, and `Scopes::insert_free` sent everything that is not `Void` down the
//! value-returning leg. That leg hoists the tail into a `__ret_N` temp precisely so the
//! tail evaluates BEFORE the frees — but only for a result type it can hoist, and
//! `never` yields no value, so nothing was hoisted and the frees went in front of the
//! tail.
//!
//! [`tests/scripts/992-match-tail-with-return-arm.loft`] carries the shape matrix, using
//! a VALUE read in the arm as its oracle. That oracle is blunt on the interpreter, where
//! a freed store keeps its bytes until something else claims them — only the text cell
//! fails there, against seven on native.
//!
//! This file uses the other oracle, and it is sharp on both: `OpDrop` prints, so the
//! RELEASE lines COUNT the frees and place them relative to the arm body. Against the
//! defect the interpreter prints `RELEASE` before the arm and again at the `return` —
//! a use-after-free — and native panics on the 65535 freed-record marker.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// A `match` in tail position whose taken arm returns. The local is a droppable, so
/// every release announces itself.
const TAKEN_ARM: &str = "struct L992 { id: integer }\n\
fn OpDrop(self: L992) { println(\"RELEASE {self.id}\"); }\n\
fn take992(id: integer) -> L992 { return L992 { id: id }; }\n\
fn d992() { x = take992(32); match 0 { 0 => { println(\"arm {x.id}\"); return }, _ => { } } }\n\
fn main() { d992(); println(\"done\"); }\n";

/// The same with the return in the arm that is NOT taken — the arm that runs must still
/// see a live value, and the release must follow it.
const UNTAKEN_ARM: &str = "struct L992 { id: integer }\n\
fn OpDrop(self: L992) { println(\"RELEASE {self.id}\"); }\n\
fn take992(id: integer) -> L992 { return L992 { id: id }; }\n\
fn d992() { x = take992(32); match 0 { 0 => { println(\"arm {x.id}\") }, _ => { return } } }\n\
fn main() { d992(); println(\"done\"); }\n";

/// Every arm returns, so the block genuinely never completes. Each arm frees for itself;
/// nothing may free ahead of them.
const EVERY_ARM: &str = "struct L992 { id: integer }\n\
fn OpDrop(self: L992) { println(\"RELEASE {self.id}\"); }\n\
fn take992(id: integer) -> L992 { return L992 { id: id }; }\n\
fn d992() { x = take992(32); match 0 { 0 => { println(\"arm {x.id}\"); return }, _ => { return } } }\n\
fn main() { d992(); println(\"done\"); }\n";

fn write_probe(tag: &str, src: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("loft_992_{tag}_{}.loft", std::process::id()));
    fa::write(&path, src).expect("write probe");
    path
}

fn run(backend: &str, file: &PathBuf) -> (bool, String, String) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(file)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// One release, and it comes after the arm body.
fn assert_released_once_after_the_arm(tag: &str, src: &str, backend: &str) {
    let probe = write_probe(&format!("{tag}_{}", backend.trim_start_matches('-')), src);
    let (ok, stdout, stderr) = run(backend, &probe);
    let _ = fa::remove_file(&probe);
    assert!(
        ok,
        "[{tag}/{backend}] the program must run to completion — a second release of the \
         same record is a use-after-free, which native reports as an out-of-bounds index \
         on the 65535 freed-record marker\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let releases = stdout.matches("RELEASE 32").count();
    assert_eq!(
        releases, 1,
        "[{tag}/{backend}] the local owns one record and must be released exactly once\n\
         stdout:\n{stdout}"
    );
    let arm = stdout.find("arm 32").unwrap_or_else(|| {
        panic!("[{tag}/{backend}] the arm must read a LIVE value\nstdout:\n{stdout}")
    });
    let release = stdout.find("RELEASE 32").expect("release line present");
    assert!(
        release > arm,
        "[{tag}/{backend}] the release must follow the arm that reads the value, not \
         precede it\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("done"),
        "[{tag}/{backend}] the caller must resume after the call\nstdout:\n{stdout}"
    );
}

#[test]
fn taken_return_arm_releases_once_interpret() {
    assert_released_once_after_the_arm("taken", TAKEN_ARM, "--interpret");
}

#[test]
fn taken_return_arm_releases_once_native() {
    assert_released_once_after_the_arm("taken", TAKEN_ARM, "--native");
}

#[test]
fn untaken_return_arm_releases_once_interpret() {
    assert_released_once_after_the_arm("untaken", UNTAKEN_ARM, "--interpret");
}

#[test]
fn untaken_return_arm_releases_once_native() {
    assert_released_once_after_the_arm("untaken", UNTAKEN_ARM, "--native");
}

#[test]
fn return_in_every_arm_releases_once_interpret() {
    assert_released_once_after_the_arm("every", EVERY_ARM, "--interpret");
}

#[test]
fn return_in_every_arm_releases_once_native() {
    assert_released_once_after_the_arm("every", EVERY_ARM, "--native");
}

/// loft#2626 — an arm whose value is a nested `match` answering `null` or returning on every
/// path, beside an arm that answers a record, text or vector.  The interpreter's chain was
/// sized from the untyped arm and never stored the sibling's value (`null`, or a refused free
/// of a stray store); the `if` form replaced the nested `match` — its `return` included — with
/// the typed null on both backends.  Each program prints `<flt> <int>`, hand-computed: the
/// `TyFlt` call answers the sibling's value, the `TyInt` call the nested `return`.
const NULL_ARM_HEAD: &str = "enum Ty2626 { TyInt { lo: integer }, TyFlt, TySgl }\n\
struct Nd2626 { name: text }\n\
fn cl2626(n: text) -> Nd2626 { Nd2626 { name: n } }\n";

const NULL_ARM_CASES: [(&str, &str, &str, &str); 5] = [
    (
        "record",
        "fn wa(at: Ty2626, pt: Ty2626) -> Nd2626? {\n  match at {\n    TyInt { lo: _ } => match pt { TyInt { lo: d } => { if d < 0 { return cl2626(\"narrow\"); } null }, _ => null },\n    TyFlt => cl2626(\"cast\"),\n    _ => null\n  }\n}\n",
        "a = wa(Ty2626::TyFlt, Ty2626::TySgl); b = wa(Ty2626::TyInt { lo: 0 - 5 }, Ty2626::TyInt { lo: 0 - 5 }); println(\"{if a == null { \"null\" } else { a!.name }} {if b == null { \"null\" } else { b!.name }}\");",
        "cast narrow",
    ),
    (
        "text",
        "fn wa(at: Ty2626, pt: Ty2626) -> text? {\n  match at {\n    TyInt { lo: _ } => match pt { TyInt { lo: d } => { if d < 0 { return \"narrow\"; } null }, _ => null },\n    TyFlt => \"cast\",\n    _ => null\n  }\n}\n",
        "a = wa(Ty2626::TyFlt, Ty2626::TySgl); b = wa(Ty2626::TyInt { lo: 0 - 5 }, Ty2626::TyInt { lo: 0 - 5 }); println(\"{a} {b}\");",
        "cast narrow",
    ),
    (
        "vector",
        "fn wa(at: Ty2626, pt: Ty2626) -> vector<integer>? {\n  match at {\n    TyInt { lo: _ } => match pt { TyInt { lo: d } => { if d < 0 { return [1]; } null }, _ => null },\n    TyFlt => [2, 3],\n    _ => null\n  }\n}\n",
        "a = wa(Ty2626::TyFlt, Ty2626::TySgl); b = wa(Ty2626::TyInt { lo: 0 - 5 }, Ty2626::TyInt { lo: 0 - 5 }); println(\"{a} {b}\");",
        "[2,3] [1]",
    ),
    (
        "bound",
        "fn wa(at: Ty2626, pt: Ty2626) -> Nd2626? {\n  r = match at {\n    TyInt { lo: _ } => match pt { TyInt { lo: d } => { if d < 0 { return cl2626(\"narrow\"); } null }, _ => null },\n    TyFlt => cl2626(\"cast\"),\n    _ => null\n  };\n  r\n}\n",
        "a = wa(Ty2626::TyFlt, Ty2626::TySgl); b = wa(Ty2626::TyInt { lo: 0 - 5 }, Ty2626::TyInt { lo: 0 - 5 }); println(\"{if a == null { \"null\" } else { a!.name }} {if b == null { \"null\" } else { b!.name }}\");",
        "cast narrow",
    ),
    (
        "if",
        "fn wa(at: Ty2626, pt: Ty2626) -> Nd2626? {\n  if at is TyInt { match pt { TyInt { lo: d } => { if d < 0 { return cl2626(\"narrow\"); } null }, _ => null } } else if at is TyFlt { cl2626(\"cast\") } else { null }\n}\n",
        "a = wa(Ty2626::TyFlt, Ty2626::TySgl); b = wa(Ty2626::TyInt { lo: 0 - 5 }, Ty2626::TyInt { lo: 0 - 5 }); println(\"{if a == null { \"null\" } else { a!.name }} {if b == null { \"null\" } else { b!.name }}\");",
        "cast narrow",
    ),
];

fn assert_null_arm_cases(backend: &str) {
    for (tag, wa, body, want) in NULL_ARM_CASES {
        let src = format!("{NULL_ARM_HEAD}{wa}fn main() {{ {body} }}\n");
        let probe = write_probe(
            &format!("2626_{tag}_{}", backend.trim_start_matches('-')),
            &src,
        );
        let (ok, stdout, stderr) = run(backend, &probe);
        let _ = fa::remove_file(&probe);
        assert!(
            ok,
            "[2626 {tag}/{backend}] must run cleanly\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert_eq!(
            stdout.trim(),
            want,
            "[2626 {tag}/{backend}]\nstderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("BUG"),
            "[2626 {tag}/{backend}] a refused free\nstderr:\n{stderr}"
        );
    }
}

#[test]
fn a_null_arm_beside_a_value_arm_keeps_both_interpret() {
    assert_null_arm_cases("--interpret");
}

#[test]
fn a_null_arm_beside_a_value_arm_keeps_both_native_c() {
    assert_null_arm_cases("--native-c");
}

/// loft#2627 — a local bound under ONE name in two sibling blocks, each time a view of a
/// different value, keeps one dep list (its last-parsed bind's).  The interpreter's rebind
/// read that list to ask "does the right-hand side read a view of `b`?", answered no, and
/// freed `b` before `b = Ex { items: [it] }` copied out of `it`.  Each program prints its
/// hand-computed answer; the interpreter answered `new null []` (the `if` form) or garbage
/// integers (the `match`-capture form).
const SIBLING_VIEW_CASES: [(&str, &str, &str); 2] = [
    (
        "if",
        "struct In2627 { name: text, ops: vector<integer> }\n\
         struct Ex2627 { items: vector<In2627>, tp: text }\n\
         fn mk2627(s: text) -> Ex2627 { Ex2627 { items: [In2627 { name: s, ops: [1, 2, 3] }], tp: \"orig\" } }\n\
         fn f(other: Ex2627, c: boolean) -> Ex2627 {\n  b = mk2627(\"first\");\n  if c {\n    it = b.items[0];\n    it.ops += [4];\n    b = Ex2627 { items: [it], tp: \"new\" };\n  }\n  if c {\n    it = other.items[0];\n    println(\"second {it.name}\");\n  }\n  b\n}\n\
         fn main() { r = f(mk2627(\"other\"), true); println(\"{r.tp} {r.items[0].name} {r.items[0].ops}\"); }\n",
        "second other\nnew first [1,2,3,4]",
    ),
    (
        "match",
        "struct Blk2627 { name: text, ops: vector<integer> }\n\
         enum Nd2627 { NBlock { block: vector<Blk2627> }, NInt { n: integer } }\n\
         struct Ex2627 { code: Nd2627, tp: text }\n\
         fn ex2627(c: Nd2627, t: text) -> Ex2627 { Ex2627 { code: c, tp: t } }\n\
         fn f(code: Nd2627) -> Ex2627 {\n  b = ex2627(Nd2627::NBlock { block: [Blk2627 { name: \"first\", ops: [1, 2, 3] }] }, \"orig\");\n  match b.code {\n    NBlock { block } => {\n      bl = block[0];\n      n = len(bl.ops);\n      bl.ops = bl.ops[0..n - 1] + [70, 80];\n      b = ex2627(Nd2627::NBlock { block: [bl] }, \"new\");\n    },\n    _ => {}\n  }\n  match code {\n    NBlock { block } => {\n      bl = block[0];\n      println(\"second {bl.name}\");\n    },\n    _ => {}\n  }\n  b\n}\n\
         fn main() {\n  r = f(Nd2627::NBlock { block: [Blk2627 { name: \"other\", ops: [9] }] });\n  match r.code { NBlock { block } => println(\"{r.tp} {block[0].name} {block[0].ops}\"), _ => println(\"other\") }\n}\n",
        "second other\nnew first [1,2,70,80]",
    ),
];

fn assert_sibling_view_cases(backend: &str) {
    for (tag, src, want) in SIBLING_VIEW_CASES {
        let probe = write_probe(
            &format!("2627_{tag}_{}", backend.trim_start_matches('-')),
            src,
        );
        let (ok, stdout, stderr) = run(backend, &probe);
        let _ = fa::remove_file(&probe);
        assert!(
            ok,
            "[2627 {tag}/{backend}] must run cleanly\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert_eq!(
            stdout.trim(),
            want,
            "[2627 {tag}/{backend}]\nstderr:\n{stderr}"
        );
    }
}

#[test]
fn a_view_bound_twice_under_one_name_keeps_its_base_interpret() {
    assert_sibling_view_cases("--interpret");
}

#[test]
fn a_view_bound_twice_under_one_name_keeps_its_base_native_c() {
    assert_sibling_view_cases("--native-c");
}

/// loft#2629 (`@FR-R-TailSelf`) — a self tail call written as an explicit `return` inside an
/// arm of a function whose tail is a `match` releases the frame ONCE on its jump.  The jump
/// took the releases before the `return` (already the whole exit) and appended the `match`
/// tail's too, so the interpreter freed every local twice ("a store was freed twice").
/// Values hand-computed: the last pass has `n == 0`.
const TAIL_RETURN_IN_ARM: &str = "struct Rg2629 { lo: integer, name: text }\n\
enum Nd2629 { NA { v: integer }, NB }\n\
fn mk2629(v: integer) -> Rg2629 { Rg2629 { lo: v, name: \"r{v}\" } }\n\
fn r2629(n: integer, c: Nd2629) -> Rg2629? {\n  p = mk2629(n);\n  match c {\n    NA { v } => {\n      if n > 0 { return r2629(n - 1, c); }\n      Rg2629 { lo: p.lo + v, name: p.name }\n    },\n    _ => null\n  }\n}\n\
fn deep2629(n: integer, c: Nd2629) -> Rg2629? {\n  p = mk2629(n);\n  match c {\n    NA { v } => {\n      if n > 0 { if v > 0 { return deep2629(n - 1, c); } }\n      Rg2629 { lo: p.lo + v, name: p.name }\n    },\n    _ => null\n  }\n}\n\
fn txt2629(n: integer, c: Nd2629) -> text {\n  s = \"s{n}\";\n  match c {\n    NA { v } => {\n      if n > 0 { return txt2629(n - 1, c); }\n      \"{s}/{v}\"\n    },\n    _ => \"\"\n  }\n}\n\
fn main() {\n  x = r2629(3, Nd2629::NA { v: 10 });\n  d = deep2629(2, Nd2629::NA { v: 5 });\n  println(\"{x!.lo} {x!.name} {d!.lo} {d!.name} {txt2629(3, Nd2629::NA { v: 7 })}\");\n}\n";

fn assert_tail_return_in_arm(backend: &str) {
    let probe = write_probe(
        &format!("2629_{}", backend.trim_start_matches('-')),
        TAIL_RETURN_IN_ARM,
    );
    let (ok, stdout, stderr) = run(backend, &probe);
    let _ = fa::remove_file(&probe);
    assert!(
        ok,
        "[2629/{backend}] must run cleanly\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        stdout.trim(),
        "10 r0 5 r0 s0/7",
        "[2629/{backend}]\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("freed twice"),
        "[2629/{backend}] a double free\nstderr:\n{stderr}"
    );
}

#[test]
fn a_self_tail_return_in_an_arm_releases_the_frame_once_interpret() {
    assert_tail_return_in_arm("--interpret");
}

#[test]
fn a_self_tail_return_in_an_arm_releases_the_frame_once_native_c() {
    assert_tail_return_in_arm("--native-c");
}
