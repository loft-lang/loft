// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN119 — THE GATE.
//
// The plan rests on one invariant:
//
//   A call to a library is indistinguishable — in type, effect,
//   ownership/lifetime, and error behaviour — from the same call in-process.
//   Where it runs is deployment policy, not source.
//
// So the test is not "does a placed call return the right number" (that is
// `placement_worker.rs`); it is "does flipping ONE LINE OF MANIFEST change
// anything observable". One unchanged consumer, one unchanged library, run
// under `placement = "inproc"` and `placement = "process"`, requiring identical
// stdout, identical stderr, and identical exit status.
//
// stderr carries the leak half of the gate: `check_store_leaks` runs on
// `--interpret` and prints there, so a placed call that leaked a store — or
// freed one the caller still owned — shows up as a stderr difference rather
// than needing a separate instrument.
//
// Any divergence here falsifies the invariant, which is exactly what it is for.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = base.join("loft-placement-parity").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// The scratch path for `name` WITHOUT wiping it — for a second look at a layout
/// `both_placements` has already built.
fn scratch_path(name: &str) -> PathBuf {
    std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("loft-placement-parity")
        .join(name)
}

/// Write the library with `placement` set to `mode`, leaving its source alone.
fn write_library(root: &Path, mode: &str, source: &str) {
    let pkg = root.join("libs").join("parity");
    std::fs::create_dir_all(pkg.join("src")).expect("create package");
    std::fs::write(
        pkg.join("loft.toml"),
        format!(
            "[package]\nname = \"parity\"\nversion = \"0.1.0\"\n\n\
             [library]\nplacement = \"{mode}\"\n"
        ),
    )
    .expect("write manifest");
    std::fs::write(pkg.join("src").join("parity.loft"), source).expect("write source");
}

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run(root: &Path, consumer: &Path) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg("--lib")
        .arg(root.join("libs"))
        .arg(consumer)
        // Bound the run: a wire that never answers would otherwise hang the suite.
        .env("LOFT_TIMEOUT", "60")
        // Vary ONE axis. Left alone, the in-process side auto-compiles the
        // library to a cdylib and the placed side does not (a worker holds the
        // loft source), so the two runs would differ in whether a native build
        // ran at all — and any chatter from that build reads as a placement
        // difference when it is nothing of the kind. Pinning both to the
        // interpreter makes placement the only thing that changed, which is what
        // the invariant is about.
        .env("LOFT_NO_NATIVE_LIBS", "1")
        .output()
        .expect("failed to invoke loft");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code().unwrap_or(-1),
    }
}

/// Run `source` + `consumer` under both placements and return the two results.
fn both_placements(name: &str, source: &str, consumer_src: &str) -> (Run, Run) {
    let root = scratch(name);
    let consumer = root.join("consumer.loft");
    std::fs::write(&consumer, consumer_src).expect("write consumer");

    write_library(&root, "inproc", source);
    let inproc = run(&root, &consumer);
    write_library(&root, "process", source);
    let placed = run(&root, &consumer);
    (inproc, placed)
}

fn assert_indistinguishable(what: &str, inproc: &Run, placed: &Run) {
    assert_eq!(
        inproc.stdout, placed.stdout,
        "{what}: placement changed the program's OUTPUT\n\
         --- inproc ---\n{}\n--- process ---\n{}",
        inproc.stdout, placed.stdout
    );
    assert_eq!(
        inproc.stderr, placed.stderr,
        "{what}: placement changed what was reported on stderr (the leak half of \
         the gate)\n--- inproc ---\n{}\n--- process ---\n{}",
        inproc.stderr, placed.stderr
    );
    assert_eq!(
        inproc.code, placed.code,
        "{what}: placement changed the exit status ({} vs {})",
        inproc.code, placed.code
    );
}

#[test]
fn placement_changes_nothing_observable() {
    let library = "pub fn add(a: integer, b: integer) -> integer {\n    a + b\n}\n\
                   pub fn tally(label: text, n: integer) -> integer {\n    len(label) + n\n}\n\
                   pub fn flag(on: boolean) -> boolean {\n    !on\n}\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   println(\"add   = {add(2, 3)}\");\n\
                    \x20   println(\"edge  = {add(-9007199254740993, 1)}\");\n\
                    \x20   println(\"tally = {tally(\"héllo\", 10)}\");\n\
                    \x20   println(\"flag  = {flag(true)}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("basic", library, consumer);
    assert_eq!(
        inproc.code, 0,
        "the in-process run must succeed: {}",
        inproc.stderr
    );
    assert_indistinguishable("scalar and text calls", &inproc, &placed);

    // Prove the gate is measuring something: the run really did produce output,
    // so "identical" is not two empty strings agreeing.
    assert!(
        inproc.stdout.contains("add   = 5"),
        "the consumer did not run as expected: {:?}",
        inproc.stdout
    );
}

#[test]
fn a_call_in_a_loop_keeps_its_answer() {
    // State and repetition: the worker holds the library across calls, so an
    // accumulating loop is where a stale frame or a mismatched response would
    // show up as a wrong total rather than a crash.
    let library = "pub fn step(n: integer) -> integer {\n    n * 2 + 1\n}\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   acc = 0;\n\
                    \x20   for i in 0..50 {\n\
                    \x20       acc += step(i);\n\
                    \x20   }\n\
                    \x20   println(\"acc = {acc}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("loop", library, consumer);
    assert_eq!(
        inproc.code, 0,
        "the in-process run must succeed: {}",
        inproc.stderr
    );
    assert_indistinguishable("repeated calls", &inproc, &placed);
    assert!(
        inproc.stdout.contains("acc = 2500"),
        "expected the hand-computed total 2500, got {:?}",
        inproc.stdout
    );
}

/// Narrow integers, at the edges of every declared width.
///
/// These crossed WRONG before @PLN119 arc B: the marshal wrote a narrow value
/// at its storage width into a stack cell that is eight bytes wide, so `-1` as
/// an `i8` came back as `255` and a `u8` argument inherited whatever the
/// previous call had left in the upper seven bytes.  The edges are the point —
/// a sweep of small positive numbers passes on a marshal that gets sign and
/// width entirely wrong.
///
/// `i32`'s low edge is `-2147483647`, not `-2147483648`: `i32::MIN` is that type's null
/// sentinel, so it is not a value an `i32` can carry (an `i32` FIELD given it reads back
/// `null`).  Its siblings differ here — a non-null `i8` does hold `-128` — which is why
/// each low edge is spelled out rather than derived.  The narrowing gate says so since
/// loft#931; before that the constant was accepted and crossed as `null`.
#[test]
fn every_integer_width_crosses_with_its_sign() {
    let library = "pub fn e_u8(v: u8) -> u8 { v }\n\
                   pub fn e_i8(v: i8) -> i8 { v }\n\
                   pub fn e_u16(v: u16) -> u16 { v }\n\
                   pub fn e_i16(v: i16) -> i16 { v }\n\
                   pub fn e_u32(v: u32) -> u32 { v }\n\
                   pub fn e_i32(v: i32) -> i32 { v }\n\
                   pub fn mixed(a: u8, b: boolean, c: text, d: i16, e: integer) -> integer {\n\
                   \x20   r = e + d + a + len(c);\n\
                   \x20   if b { r += 1000; }\n\
                   \x20   r\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   println(\"u8  {e_u8(255)} {e_u8(0)}\");\n\
                    \x20   println(\"i8  {e_i8(-128)} {e_i8(-1)} {e_i8(127)}\");\n\
                    \x20   println(\"u16 {e_u16(65535)}\");\n\
                    \x20   println(\"i16 {e_i16(-32768)} {e_i16(-1)}\");\n\
                    \x20   println(\"u32 {e_u32(4294967294)}\");\n\
                    \x20   println(\"i32 {e_i32(-2147483647)} {e_i32(-1)}\");\n\
                    \x20   println(\"mix {mixed(200, true, \"abc\", -9, 7)}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("narrow", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("narrow integers", &inproc, &placed);
    // Hand-computed, so this is not just two runs agreeing on a wrong answer:
    // 7 + (-9) + 200 + 3 + 1000 = 1201.
    assert!(
        inproc.stdout.contains("i8  -128 -1 127") && inproc.stdout.contains("mix 1201"),
        "the hand-computed values are not what ran: {:?}",
        inproc.stdout
    );
}

/// `single` and a text RETURN — the two shapes arc A refused rather than
/// approximate.
///
/// The text return is the interesting one: it does not come back on the stack,
/// and the call site picks between two conventions depending on where the
/// result is going.  So this exercises both — assigned to a variable, and used
/// in a value position (inside a format string) — plus nesting one call in
/// another, reassigning the same variable, and a loop, because a destination
/// that leaked or was reused would cross two answers rather than lose one.
#[test]
fn single_and_text_returns_cross() {
    let library = "pub fn echo_single(v: single) -> single { v }\n\
                   pub fn scale(v: single, by: integer) -> single { v * by }\n\
                   pub fn shout(s: text) -> text { \"<{s}>\" }\n\
                   pub fn tag(s: text) -> text { \"[{s}]\" }\n\
                   pub fn grow(n: integer) -> text {\n\
                   \x20   out = \"\";\n\
                   \x20   for i in 0..n { out += \"a{i * 0}\"; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn mix(a: integer, b: boolean, c: text, d: single, e: integer) -> text {\n\
                   \x20   \"{a}|{b}|{c}|{d}|{e}\"\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   println(\"s1 {echo_single(0.5 as single)} {echo_single(-2.25 as single)}\");\n\
                    \x20   println(\"s2 {scale(1.5 as single, 4)}\");\n\
                    \x20   v = shout(\"hi\");\n\
                    \x20   println(\"assigned {v}\");\n\
                    \x20   println(\"inline {shout(\"hi\")}\");\n\
                    \x20   println(\"unicode {shout(\"héllo ✓\")}\");\n\
                    \x20   println(\"nested {tag(shout(\"x\"))}\");\n\
                    \x20   println(\"twice {tag(\"a\")}{tag(\"b\")}\");\n\
                    \x20   println(\"long {len(grow(300))}\");\n\
                    \x20   println(\"mix {mix(7, true, \"cc\", 0.5 as single, -9)}\");\n\
                    \x20   r = shout(\"1\");\n\
                    \x20   r = shout(\"2\");\n\
                    \x20   println(\"reassigned {r}\");\n\
                    \x20   acc = \"\";\n\
                    \x20   for i in 0..3 { acc += tag(\"{i}\"); }\n\
                    \x20   println(\"loop {acc}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("single_text", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("single and text returns", &inproc, &placed);
    for expect in [
        "s1 0.5 -2.25",
        "assigned <hi>",
        "inline <hi>",
        "nested [<x>]",
        "twice [a][b]",
        "long 600",
        "mix 7|true|cc|0.5|-9",
        "reassigned <2>",
        "loop [0][1][2]",
    ] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?} in the output, got {:?}",
            inproc.stdout
        );
    }
}

/// A text return the compiler never gave a work buffer — the usual case being a
/// constant, `fn version() -> text { "1.0" }`.
///
/// The caller offers nowhere for such an answer to live, so the function is not
/// placed and runs in-process.  What must hold is that this is INVISIBLE: the
/// program behaves identically either way, which is the whole claim placement
/// makes.  Without the refusal the dispatcher would push a `Str` over a String
/// freed the moment the call returned.
#[test]
fn a_text_return_with_no_work_buffer_still_behaves_identically() {
    let library = "pub fn version() -> text { \"1.0.0\" }\n\
                   pub fn add(a: integer, b: integer) -> integer { a + b }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   println(\"v = {version()}\");\n\
                    \x20   w = version();\n\
                    \x20   println(\"w = {w} / {add(1, 2)}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("constret", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("a constant text return", &inproc, &placed);
    assert!(
        inproc.stdout.contains("v = 1.0.0") && inproc.stdout.contains("w = 1.0.0 / 3"),
        "{:?}",
        inproc.stdout
    );
}

#[test]
fn a_warning_in_the_library_does_not_decide_whether_it_can_be_placed() {
    // A library that is CORRECT but not diagnostic-free. The worker loads it
    // through `parse_dir`, which used to refuse a directory whose parse
    // reported ANYTHING — so one `never-read` warning made the consumer exit 1
    // under `placement = "process"` and 0 in-process, i.e. placement decided
    // whether the program ran at all. Errors still stop the load; warnings and
    // advice never did gate anything else in loft, and now do not gate this.
    let library = "pub fn ok(a: integer, unused: integer) -> integer {\n    a * 2\n}\n";
    let consumer = "use parity::*;\nfn main() {\n    println(\"ok = {ok(21, 5)}\");\n}\n";
    let (inproc, placed) = both_placements("warned", library, consumer);
    assert_eq!(
        inproc.code, 0,
        "the in-process run must succeed: {}",
        inproc.stderr
    );
    // The warning has to be REAL, or this test would pass on a library that never had one
    // — the same blindness `the_gate_can_fail` guards against.  It is not read off the
    // CONSUMER's run: `parity` is a dependency here, with its own manifest in its own
    // directory, and loft#1260 addresses a lint to whoever can act on its cure, so the
    // consumer is correctly not told about a `never-read` inside it.  Ask where it IS
    // addressed — the library compiled as its own entry.
    let alone = run(
        &scratch_path("warned"),
        &scratch_path("warned")
            .join("libs")
            .join("parity")
            .join("src")
            .join("parity.loft"),
    );
    assert!(
        alone.stderr.contains("never read"),
        "the probe library no longer warns, so it proves nothing: {:?}",
        alone.stderr
    );
    assert!(
        !inproc.stderr.contains("never read"),
        "a dependency's warning must not reach its consumer (loft#1260): {:?}",
        inproc.stderr
    );
    assert_indistinguishable("a library that warns", &inproc, &placed);
    assert!(inproc.stdout.contains("ok = 42"), "{:?}", inproc.stdout);
}

/// `--native` runs a placed library IN-PROCESS, and saying so is the point.
///
/// That backend compiles the library's own body into the whole-program binary,
/// so its calls never leave the process however they were marked.  By the
/// invariant that is the same PROGRAM — but it is not the same ISOLATION, and a
/// deployment that declared `placement = "process"` to contain a crash would
/// have got no worker and no trace of it in any output.  (It was worse than
/// silent: a worker was started per library and then never called.)
///
/// `LOFT_REQUIRE_PLACEMENT=1` is how a deployment insists, so it must refuse
/// here for the same reason it refuses on a platform with no transport.
/// One library, some functions placed and some not — which is the ordinary case,
/// not an edge one, because a real library has signatures on both sides of the
/// line (a struct, a constant text return).
///
/// The two kinds of call sit next to each other on the caller's stack, so this
/// is where a dispatcher that popped the wrong number of cells would show up:
/// not as a failure at the placed call, but as a wrong value in whatever ran
/// next.  Hence the interleaving, and a placed call whose ARGUMENTS come from an
/// unplaceable one.
#[test]
fn placed_and_unplaceable_calls_interleave() {
    let library = "pub fn placed_add(a: integer, b: integer) -> integer { a + b }\n\
                   pub fn version() -> text { \"1.0.0\" }\n\
                   pub struct Point { x: integer, y: integer }\n\
                   pub fn make_point(x: integer, y: integer) -> Point { Point { x: x, y: y } }\n\
                   pub fn sum_point(p: Point) -> integer { p.x + p.y }\n\
                   pub fn placed_txt(s: text) -> text { \"<{s}>\" }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   p = make_point(3, 4);\n\
                    \x20   println(\"a {placed_add(1, 2)} {version()} {sum_point(p)} {placed_txt(\"z\")}\");\n\
                    \x20   println(\"b {placed_add(sum_point(p), len(version()))}\");\n\
                    \x20   println(\"c {placed_txt(version())}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("mixed", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("mixed placeable and unplaceable", &inproc, &placed);
    // Hand-computed: sum_point = 7, len("1.0.0") = 5, so b = 12.
    for expect in ["a 3 1.0.0 7 <z>", "b 12", "c <1.0.0>"] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

/// @PLN119 arc B — every compound shape, both directions.
///
/// A struct or a vector does not fit in a call frame: it is a graph of records,
/// and it crosses through the shared arena.  The rows here are the composition
/// matrix's type-kind axis, and the ones that matter are the ones whose bytes
/// are NOT plain words in the record — a `text` field is a pointer to a
/// sub-record, a `vector` is a pointer to an element block, and both have to be
/// re-homed into the arena's own store or the receiving side reads a rec-id that
/// means something else over there.
///
/// `vector<text>` is here on purpose: @PLN97's paged relocator REFUSES it,
/// because that path moves a matched entry into a different store and each
/// element's pointer would dangle.  The arena is not that path — both sides map
/// the same store — so the refusal does not apply, and this is what says so.
#[test]
fn compound_values_cross_in_both_directions() {
    let library = "pub struct P { x: integer, y: integer }\n\
                   pub struct N { id: integer, label: text }\n\
                   pub struct O { tag: text, inner: P, tail: vector<integer> }\n\
                   pub fn sum_p(p: P) -> integer { p.x + p.y }\n\
                   pub fn make_p(x: integer, y: integer) -> P { P { x: x, y: y } }\n\
                   pub fn describe(n: N) -> text { \"{n.id}:{n.label}\" }\n\
                   pub fn make_n(id: integer, label: text) -> N { N { id: id, label: label } }\n\
                   pub fn sum_v(v: vector<integer>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for e in v { t += e; }\n\
                   \x20   t\n\
                   }\n\
                   pub fn range_v(n: integer) -> vector<integer> {\n\
                   \x20   out: vector<integer> = [];\n\
                   \x20   for i in 0..n { out += [i]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn make_o(tag: text, n: integer) -> O {\n\
                   \x20   v: vector<integer> = [];\n\
                   \x20   for i in 0..n { v += [i * 2]; }\n\
                   \x20   O { tag: tag, inner: P { x: n, y: n * 10 }, tail: v }\n\
                   }\n\
                   pub fn show_o(o: O) -> text {\n\
                   \x20   t = 0;\n\
                   \x20   for e in o.tail { t += e; }\n\
                   \x20   \"{o.tag}|{o.inner.x + o.inner.y}|{t}|{len(o.tail)}\"\n\
                   }\n\
                   pub fn names(n: integer) -> vector<N> {\n\
                   \x20   out: vector<N> = [];\n\
                   \x20   for i in 0..n { out += [N { id: i, label: \"n{i}\" }]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn join_names(v: vector<N>) -> text {\n\
                   \x20   s = \"\";\n\
                   \x20   for e in v { s += \"{e.id}={e.label};\"; }\n\
                   \x20   s\n\
                   }\n\
                   pub fn words(n: integer) -> vector<text> {\n\
                   \x20   out: vector<text> = [];\n\
                   \x20   for i in 0..n { out += [\"w{i}\"]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn join_words(v: vector<text>) -> text {\n\
                   \x20   s = \"\";\n\
                   \x20   for e in v { s += \"{e},\"; }\n\
                   \x20   s\n\
                   }\n\
                   pub fn maybe(p: P?) -> integer { if p == null { -1 } else { p.x + p.y } }\n\
                   pub fn nested(n: integer) -> vector<vector<integer>> {\n\
                   \x20   out: vector<vector<integer>> = [];\n\
                   \x20   for i in 0..n {\n\
                   \x20       row: vector<integer> = [];\n\
                   \x20       for j in 0..n { row += [i * 10 + j]; }\n\
                   \x20       out += [row];\n\
                   \x20   }\n\
                   \x20   out\n\
                   }\n\
                   pub fn sum_nested(v: vector<vector<integer>>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for row in v { for e in row { t += e; } }\n\
                   \x20   t\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   p = make_p(3, 4);\n\
                    \x20   println(\"struct {sum_p(p)} {p.x} {p.y}\");\n\
                    \x20   n = make_n(7, \"héllo\");\n\
                    \x20   println(\"text-field {describe(n)} / {n.id} {n.label}\");\n\
                    \x20   v = range_v(5);\n\
                    \x20   println(\"vector {sum_v(v)} {len(v)} {v[4]}\");\n\
                    \x20   o = make_o(\"tag\", 4);\n\
                    \x20   println(\"nested-struct {show_o(o)} / {o.inner.y} {o.tail[3]}\");\n\
                    \x20   vn = names(3);\n\
                    \x20   println(\"vec-struct {join_names(vn)} {len(vn)} {vn[1].label}\");\n\
                    \x20   vw = words(3);\n\
                    \x20   println(\"vec-text {join_words(vw)} {len(vw)} {vw[2]}\");\n\
                    \x20   empty: vector<integer> = [];\n\
                    \x20   println(\"empty {sum_v(empty)} {len(empty)}\");\n\
                    \x20   absent: P? = null;\n\
                    \x20   println(\"null {maybe(p)} {maybe(absent)}\");\n\
                    \x20   g = nested(3);\n\
                    \x20   println(\"depth2 {sum_nested(g)} {len(g)} {g[1][1]}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("compound", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("compound values", &inproc, &placed);
    // Hand-computed, so this is not two runs agreeing on a wrong answer:
    // sum_p = 7; sum_v(0..5) = 10; show_o("tag",4) = inner 4+40 = 44, tail
    // 0+2+4+6 = 12, len 4; nested(3) sums 0+1+2 + 10+11+12 + 20+21+22 = 99.
    for expect in [
        "struct 7 3 4",
        "text-field 7:héllo / 7 héllo",
        "vector 10 5 4",
        "nested-struct tag|44|12|4 / 40 6",
        "vec-struct 0=n0;1=n1;2=n2; 3 n1",
        "vec-text w0,w1,w2, 3 w2",
        "empty 0 0",
        "null 7 -1",
        "depth2 99 3 11",
    ] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

/// The cell a copy-only crossing fails.
///
/// loft passes a compound BY REFERENCE: `pub fn bump(p: P)` that assigns `p.x`
/// changes the CALLER's `p`, and a vector parameter that is appended to grows the
/// caller's vector.  A marshal that copied the argument over and stopped would
/// leave both silently unchanged under `placement = "process"` — a divergence
/// with no error, no warning, and a plausible-looking answer.
///
/// The text case is the sharper one: the field is a pointer to a sub-record, so
/// the write has to come back re-homed into the caller's store rather than as a
/// rec-id that means something else there.
#[test]
fn a_callee_writing_to_a_compound_parameter_is_seen_by_the_caller() {
    let library = "pub struct P { x: integer, label: text }\n\
                   pub fn bump(p: P) -> integer {\n\
                   \x20   p.x = p.x + 100;\n\
                   \x20   p.label = \"{p.label}!\";\n\
                   \x20   p.x\n\
                   }\n\
                   pub fn push(v: vector<integer>) -> integer {\n\
                   \x20   v += [99];\n\
                   \x20   len(v)\n\
                   }\n\
                   pub fn both(a: P, b: P) -> integer {\n\
                   \x20   a.x = a.x + 1;\n\
                   \x20   b.x = b.x + 10;\n\
                   \x20   a.x + b.x\n\
                   }\n\
                   pub fn range_v(n: integer) -> vector<integer> {\n\
                   \x20   out: vector<integer> = [];\n\
                   \x20   for i in 0..n { out += [i]; }\n\
                   \x20   out\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   p = P { x: 11, label: \"a\" };\n\
                    \x20   r = bump(p);\n\
                    \x20   println(\"write {r} caller-sees {p.x} {p.label}\");\n\
                    \x20   v = range_v(3);\n\
                    \x20   n = push(v);\n\
                    \x20   println(\"append {n} caller-sees {len(v)} {v[3]}\");\n\
                    \x20   q = P { x: 5, label: \"q\" };\n\
                    \x20   println(\"alias {both(q, q)} caller-sees {q.x}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("byref", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("a written-to compound parameter", &inproc, &placed);
    // `both(q, q)` passes ONE record twice, so the two writes compound: 5 → 6 →
    // 16, and `a.x + b.x` reads the same cell twice = 32.  Two independent
    // copies would answer 6 + 15 = 21 and leave the caller at 6 or 15.
    for expect in [
        "write 111 caller-sees 111 a!",
        "append 4 caller-sees 4 99",
        "alias 32 caller-sees 16",
    ] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

/// A value far larger than the arena's initial size, and enough repetitions that
/// a per-call leak would show.
///
/// Growth is where the two mappings can disagree: a claim that outgrows the file
/// resizes and re-mmaps it in the WRITER, and the reader's mapping still covers
/// the old length — reading past it is a `SIGBUS`, not a wrong answer.  So the
/// writer publishes its word count with every frame and the reader maps again.
///
/// The loop is the other half: the arena is reset per call rather than freed
/// record by record, so a reset that did not actually reclaim would show here as
/// a growing file rather than as a wrong value.
#[test]
fn a_value_that_outgrows_the_arena_still_crosses() {
    let library = "pub struct N { id: integer, label: text }\n\
                   pub fn range_v(n: integer) -> vector<integer> {\n\
                   \x20   out: vector<integer> = [];\n\
                   \x20   for i in 0..n { out += [i]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn sum_v(v: vector<integer>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for e in v { t += e; }\n\
                   \x20   t\n\
                   }\n\
                   pub fn names(n: integer) -> vector<N> {\n\
                   \x20   out: vector<N> = [];\n\
                   \x20   for i in 0..n { out += [N { id: i, label: \"n{i}\" }]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn count(v: vector<N>) -> integer { len(v) }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   big = range_v(200000);\n\
                    \x20   println(\"big {len(big)} {big[199999]} {sum_v(big)}\");\n\
                    \x20   ns = names(20000);\n\
                    \x20   println(\"names {count(ns)} {ns[19999].label}\");\n\
                    \x20   acc = 0;\n\
                    \x20   for i in 0..500 {\n\
                    \x20       v = range_v(50);\n\
                    \x20       acc += sum_v(v);\n\
                    \x20   }\n\
                    \x20   println(\"loop {acc}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("grow", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("a value larger than the arena", &inproc, &placed);
    // Hand-computed: sum 0..200000 = 19999900000; 500 × sum(0..50) = 500 × 1225.
    for expect in [
        "big 200000 199999 19999900000",
        "names 20000 n19999",
        "loop 612500",
    ] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

/// The leak half of the gate, made falsifiable.
///
/// `check_store_leaks` reports what is unfreed at EXIT, so it cannot see a
/// program that allocates a store per call and frees it — which is exactly the
/// shape a placed call has, and exactly the shape that runs a long program out
/// of store slots.  `LOFT_STRICT_STORES` is what makes it visible: it stops the
/// allocator recycling a released slot, so the peak slot count becomes a
/// straight count of how many stores a run ever needed, and the two placements
/// have to agree on it.
///
/// They did not.  A placed struct return minted its destination store while the
/// call arena was registered, so the arena's slots could never come back below
/// it; and adopting the arena PUSHED a slot rather than taking the one at the
/// watermark, so the table walked upward once per call.  Together that was two
/// slots per call against four for the whole in-process run — all 65535 gone
/// after ~32k iterations, with the same loop in-process flat.
///
/// Comparing the numbers rather than asserting a constant is what keeps this
/// honest: it fails if EITHER side regresses, and it cannot pass by both being
/// wrong in the same direction, because in-process placement is not doing
/// anything this plan changed.
#[test]
fn placement_does_not_change_how_many_stores_a_run_needs() {
    let library = "pub struct P { x: integer, y: integer }\n\
                   pub fn make_p(x: integer) -> P { P { x: x, y: x * 2 } }\n\
                   pub fn sum_p(p: P) -> integer { p.x + p.y }\n\
                   pub fn range_v(n: integer) -> vector<integer> {\n\
                   \x20   out: vector<integer> = [];\n\
                   \x20   for i in 0..n { out += [i]; }\n\
                   \x20   out\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   acc = 0;\n\
                    \x20   for i in 0..2000 {\n\
                    \x20       p = make_p(i);\n\
                    \x20       acc += sum_p(p);\n\
                    \x20       v = range_v(4);\n\
                    \x20       acc += len(v);\n\
                    \x20   }\n\
                    \x20   println(\"acc {acc}\");\n\
                    }\n";
    let root = scratch("slots");
    let consumer_path = root.join("consumer.loft");
    std::fs::write(&consumer_path, consumer).expect("write consumer");

    let peak_of = |mode: &str| -> (u32, String) {
        write_library(&root, mode, library);
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--interpret")
            .arg("--lib")
            .arg(root.join("libs"))
            .arg(&consumer_path)
            .env("LOFT_TIMEOUT", "120")
            .env("LOFT_NO_NATIVE_LIBS", "1")
            .env("LOFT_STRICT_STORES", "1")
            .env("LOFT_ALLOC_REPORT", "1")
            .output()
            .expect("failed to invoke loft");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let peak = stderr
            .lines()
            .find_map(|l| l.strip_prefix("loft-alloc: peak="))
            .and_then(|r| r.split_whitespace().next())
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or_else(|| panic!("no alloc report under {mode}: {stderr}"));
        (peak, String::from_utf8_lossy(&out.stdout).into_owned())
    };

    let (inproc_peak, inproc_out) = peak_of("inproc");
    let (placed_peak, placed_out) = peak_of("process");
    assert_eq!(
        inproc_out, placed_out,
        "the two runs did not even produce the same answer"
    );
    // 2000 iterations, so a per-call slot lost is a peak in the thousands. The
    // in-process side is the reference; a small constant either way would be a
    // real difference in how many stores are live AT ONCE, which is not what
    // this is measuring.
    assert!(
        placed_peak <= inproc_peak + 4,
        "placement needed {placed_peak} store slots where in-process needed \
         {inproc_peak} — a slot per call is a table that runs out"
    );
    assert!(
        inproc_peak < 100,
        "the in-process reference is itself growing ({inproc_peak}), so this \
         test is measuring nothing"
    );
}

/// A signature the arena does not carry runs in-process, and that has to be
/// INVISIBLE — which is the whole claim placement makes.
///
/// A polymorphic enum and a keyed collection are both reference-shaped, so they
/// look placeable from a distance; they are refused because their crossing has
/// questions of its own (an enum's payload type is a runtime discriminant, and a
/// keyed collection carries an index whose ordering is the caller's).  The risk
/// is not that they fail — it is that they are quietly marked and then read as
/// the wrong shape, so this pins the refusal by requiring the program to be
/// identical either way.
#[test]
fn a_compound_the_arena_does_not_carry_still_behaves_identically() {
    let library = "pub struct P { a: integer, b: integer }\n\
                   pub enum Shape { Circle { r: integer }, Square { s: integer } }\n\
                   pub fn area(sh: Shape) -> integer {\n\
                   \x20   match sh { Circle { r } => r * r * 3, Square { s } => s * s }\n\
                   }\n\
                   pub fn make_circle(r: integer) -> Shape { Circle { r: r } }\n\
                   pub fn tally(h: hash<P[a]>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for e in h { t += e.b; }\n\
                   \x20   t\n\
                   }\n\
                   pub fn sum_p(p: P) -> integer { p.a + p.b }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   c = make_circle(4);\n\
                    \x20   println(\"enum {area(c)}\");\n\
                    \x20   h: hash<P[a]> = [];\n\
                    \x20   h += [P { a: 1, b: 10 }];\n\
                    \x20   h += [P { a: 2, b: 20 }];\n\
                    \x20   println(\"hash {tally(h)}\");\n\
                    \x20   println(\"placed {sum_p(P { a: 6, b: 7 })}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("unsupported", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("an unsupported compound", &inproc, &placed);
    for expect in ["enum 48", "hash 30", "placed 13"] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

#[test]
fn native_does_not_place_and_says_so_when_asked_to_insist() {
    let root = scratch("native_require");
    let consumer = root.join("consumer.loft");
    std::fs::write(
        &consumer,
        "use parity::*;\nfn main() {\n    println(\"v = {add(2, 3)}\");\n}\n",
    )
    .expect("write consumer");
    write_library(
        &root,
        "process",
        "pub fn add(a: integer, b: integer) -> integer { a + b }\n",
    );

    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--native")
        .arg("--lib")
        .arg(root.join("libs"))
        .arg(&consumer)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_REQUIRE_PLACEMENT", "1")
        .output()
        .expect("failed to invoke loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "insisting on placement under --native must refuse; it exited {:?} with {stderr}",
        out.status.code()
    );
    assert!(
        stderr.contains("LOFT_REQUIRE_PLACEMENT") && stderr.contains("--native"),
        "the refusal must name the reason, not just fail: {stderr}"
    );
}

/// @PLN119 arc F — a placed library resolves a relative path where the CALLER
/// does.
///
/// loft anchors a program's relative file access at its own source directory and
/// chdirs there before running. Workers are started long before that, so one
/// inherited the INVOCATION directory instead — and then every relative path a
/// placed library touched resolved somewhere else than the same library
/// in-process. No error, no warning: `lib/git` simply answered "not a git
/// repository" under one placement and the history under the other.
///
/// This is the cell the whole matrix was missing, and it is worth naming why:
/// every earlier cell passes a value ACROSS the boundary, and this one asks what
/// the far side already IS. A worker inherits an environment, not only a frame.
#[test]
fn a_placed_library_sees_the_same_working_directory() {
    let library = "pub fn read_here(name: text) -> text {\n\
                   \x20   f = file(name);\n\
                   \x20   if !exists(f) { return \"MISSING\"; }\n\
                   \x20   f.content()\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   println(\"beside-me {read_here(\"marker.txt\")}\");\n\
                    }\n";
    let root = scratch("cwd");
    let consumer_path = root.join("consumer.loft");
    std::fs::write(&consumer_path, consumer).expect("write consumer");
    // The marker sits beside the CONSUMER, which is where loft anchors a
    // relative path — and is not the directory the test runner is in.
    std::fs::write(root.join("marker.txt"), "found-beside-the-program").expect("write marker");

    write_library(&root, "inproc", library);
    let inproc = run(&root, &consumer_path);
    write_library(&root, "process", library);
    let placed = run(&root, &consumer_path);

    assert!(
        inproc.stdout.contains("found-beside-the-program"),
        "the in-process reference did not find the marker, so this test is \
         measuring nothing: {:?} / {}",
        inproc.stdout,
        inproc.stderr
    );
    assert_indistinguishable("a relative path from a placed library", &inproc, &placed);
}

/// @PLN119 arc C — the @PLN94 ownership oracle over a placed program.
///
/// **Read what this proves carefully, because the obvious reading is wrong.**
/// The oracle runs inside `scopes::check`, which happens BEFORE placement marks
/// anything, so it sees byte-identical IR under both placements. Requiring the
/// two to agree would therefore be an oracle agreeing with itself — the shape
/// the plan's own method warns about. That is why arc C's ownership content is
/// the delivery-lens cross-check (`a_return_that_borrows_its_argument_is_not_placed`)
/// and not this.
///
/// What this DOES pin is worth having anyway, and it is two things:
///
/// * the oracle is clean over a program that uses a placed library at all — a
///   future change to marking that left the consumer's IR ownership-inconsistent
///   would fire here;
/// * under `process` the oracle runs in TWO processes over two programs, because
///   the worker parses and checks the library itself. Placement therefore gets
///   MORE ownership checking than in-process, not less, and both have to be
///   clean.
#[test]
fn the_ownership_oracle_is_clean_over_a_placed_program() {
    let library = "pub struct P { x: integer, label: text }\n\
                   pub fn make_v(n: integer) -> vector<P> {\n\
                   \x20   out: vector<P> = [];\n\
                   \x20   for i in 0..n { out += [P { x: i, label: \"e{i}\" }]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn make_p(x: integer) -> P { P { x: x, label: \"m{x}\" } }\n\
                   pub fn bump(p: P) -> integer {\n\
                   \x20   p.x = p.x + 1;\n\
                   \x20   p.x\n\
                   }\n\
                   pub fn sum_v(v: const vector<P>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for e in v { t += e.x; }\n\
                   \x20   t\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   v = make_v(5);\n\
                    \x20   p = make_p(3);\n\
                    \x20   println(\"o {sum_v(v)} {bump(p)} {p.x}\");\n\
                    }\n";
    let root = scratch("oracle");
    let consumer_path = root.join("consumer.loft");
    std::fs::write(&consumer_path, consumer).expect("write consumer");
    for mode in ["inproc", "process"] {
        write_library(&root, mode, library);
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--interpret")
            .arg("--lib")
            .arg(root.join("libs"))
            .arg(&consumer_path)
            .env("LOFT_TIMEOUT", "60")
            .env("LOFT_NO_NATIVE_LIBS", "1")
            .env("LOFT_OWN_ORACLE", "check")
            .output()
            .expect("failed to invoke loft");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("OWN-CHECK"),
            "the oracle did not run under {mode} — this test would pass on \
             silence: {stderr}"
        );
        let red: Vec<&str> = stderr
            .lines()
            .filter(|l| l.contains("OWN-CHECK") && !l.contains(" 0 RED") && !l.contains("clean"))
            .collect();
        assert!(
            red.is_empty(),
            "the ownership oracle reported findings under {mode}: {red:?}"
        );
        assert_eq!(
            out.status.code(),
            Some(0),
            "the run itself failed under {mode}: {stderr}"
        );
    }
}

/// @PLN119 arc C — a heap return is delivered one of THREE ways, and only two of
/// them leave the caller owning what it gets.
///
/// `fn head(v: vector<P>) -> P { v[0] }` hands back a view of the caller's own
/// argument.  In-process that is right and costs nothing: there is no new store,
/// so the caller's emitted code frees none.  Placed, the answer has to be copied
/// into the caller's address space — and a copy nobody frees is a leak PER CALL.
/// It showed up here as one line of stderr the in-process run did not have.
///
/// So a `View` return is not placed.  A view across a process boundary is not a
/// view anyway: the thing it views is in the other process.
///
/// The two placeable deliveries are here beside it, because the refusal has to
/// be the narrow one — a rule that also caught `Owned` (a constructor) or
/// `RetBuf` (a vector builder) would quietly un-place most of a library.
#[test]
fn a_return_that_borrows_its_argument_is_not_placed() {
    let library = "pub struct P { x: integer, label: text }\n\
                   pub fn head(v: vector<P>) -> P { v[0] }\n\
                   pub fn make_p(x: integer) -> P { P { x: x, label: \"m{x}\" } }\n\
                   pub fn make_v(n: integer) -> vector<P> {\n\
                   \x20   out: vector<P> = [];\n\
                   \x20   for i in 0..n { out += [P { x: i, label: \"e{i}\" }]; }\n\
                   \x20   out\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   v = make_v(4);\n\
                    \x20   acc = 0;\n\
                    \x20   for i in 0..200 {\n\
                    \x20       h = head(v);\n\
                    \x20       acc += h.x;\n\
                    \x20   }\n\
                    \x20   println(\"view {acc} {len(v)}\");\n\
                    \x20   p = make_p(9);\n\
                    \x20   println(\"owned {p.x} {p.label}\");\n\
                    \x20   println(\"retbuf {len(make_v(3))}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("borrowret", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    // The loop is what makes this bite: one leaked store per call, so a
    // divergence is a stderr line rather than a wrong number.
    assert_indistinguishable("a borrowed return", &inproc, &placed);
    assert!(
        !inproc.stderr.contains("not freed"),
        "the in-process reference itself leaks, so this proves nothing: {}",
        inproc.stderr
    );
    for expect in ["view 0 4", "owned 9 m9", "retbuf 3"] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

/// A `const` compound parameter skips the copy-back, and that must be invisible.
///
/// loft REJECTS every mutation through a `const` parameter at compile time, so
/// there is provably nothing for the copy-back to bring home — and the copy-back
/// is the expensive half of a compound crossing.  The risk in acting on that is
/// obvious: skip it where the callee COULD have written, and the caller silently
/// keeps a stale value.  So both spellings of the same function are here, and
/// the answers must match.
#[test]
fn a_const_parameter_crosses_without_a_copy_back() {
    let library = "pub struct P { x: integer, label: text }\n\
                   pub fn read_c(p: const P) -> integer { p.x * 100 + len(p.label) }\n\
                   pub fn read_m(p: P) -> integer { p.x * 100 + len(p.label) }\n\
                   pub fn sum_c(v: const vector<P>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for e in v { t += e.x; }\n\
                   \x20   t\n\
                   }\n\
                   pub fn sum_m(v: vector<P>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for e in v { t += e.x; }\n\
                   \x20   t\n\
                   }\n\
                   pub fn make_v(n: integer) -> vector<P> {\n\
                   \x20   out: vector<P> = [];\n\
                   \x20   for i in 0..n { out += [P { x: i, label: \"e{i}\" }]; }\n\
                   \x20   out\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   p = P { x: 7, label: \"abc\" };\n\
                    \x20   println(\"scalar {read_c(p)} {read_m(p)} {p.x} {p.label}\");\n\
                    \x20   v = make_v(6);\n\
                    \x20   println(\"vector {sum_c(v)} {sum_m(v)} {len(v)}\");\n\
                    \x20   acc = 0;\n\
                    \x20   for i in 0..100 { acc += sum_c(v) + read_c(p); }\n\
                    \x20   println(\"loop {acc} {len(v)} {p.x}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("constarg", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("a const compound parameter", &inproc, &placed);
    // Hand-computed: read = 7*100 + 3 = 703; sum over 0..6 = 15; loop = 100*(15+703).
    for expect in ["scalar 703 703 7 abc", "vector 15 15 6", "loop 71800 6 7"] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

/// @PLN119 arc C / Q3 — a placed library called from a `par` arm.
///
/// The wire has one request slot and the dispatcher holds a lock across the
/// whole crossing, so N workers calling one placed library serialise.  That is
/// correctness, not caution — two threads interleaving two frames in one buffer
/// would be a wrong answer, not a slow one — but "they serialise" and "they
/// deadlock" look the same from outside until someone runs it.
///
/// Both parent-sharing modes, because they are genuinely different situations
/// for a crossing: `LOFT_PAR_SHARE=1` borrows the parent's stores READ-ONLY, so
/// a copy-back aimed at one would be a write to a read-only store rather than a
/// wrong value.
#[test]
fn a_placed_call_from_a_par_arm_is_the_same_call() {
    let library = "pub struct P { x: integer, label: text }\n\
                   pub fn score(p: const P) -> integer { p.x * 11 }\n\
                   pub fn make_v(n: integer) -> vector<P> {\n\
                   \x20   out: vector<P> = [];\n\
                   \x20   for i in 0..n { out += [P { x: i, label: \"e{i}\" }]; }\n\
                   \x20   out\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   items = make_v(64);\n\
                    \x20   sum = 0;\n\
                    \x20   for a in items par(b=score(a), 4) {\n\
                    \x20       sum += b;\n\
                    \x20   }\n\
                    \x20   println(\"par {sum}\");\n\
                    }\n";
    let root = scratch("par");
    let consumer_path = root.join("consumer.loft");
    std::fs::write(&consumer_path, consumer).expect("write consumer");
    for share in ["0", "1"] {
        let go = |mode: &str| -> Run {
            write_library(&root, mode, library);
            let out = Command::new(env!("CARGO_BIN_EXE_loft"))
                .arg("--interpret")
                .arg("--lib")
                .arg(root.join("libs"))
                .arg(&consumer_path)
                .env("LOFT_TIMEOUT", "60")
                .env("LOFT_NO_NATIVE_LIBS", "1")
                .env("LOFT_PAR_SHARE", share)
                .output()
                .expect("failed to invoke loft");
            Run {
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                code: out.status.code().unwrap_or(-1),
            }
        };
        let inproc = go("inproc");
        let placed = go("process");
        assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
        assert_indistinguishable(
            &format!("a par arm (LOFT_PAR_SHARE={share})"),
            &inproc,
            &placed,
        );
        // 11 × sum(0..64) = 11 × 2016.
        assert!(
            inproc.stdout.contains("par 22176"),
            "expected the hand-computed 22176, got {:?}",
            inproc.stdout
        );
    }
}

/// A callee REBINDING its compound parameter (`v = []`) is not a write THROUGH
/// it, and must stay invisible to the caller on both placements.
///
/// The distinction matters because arc B's copy-back exists precisely to carry
/// writes home, and a marshal that copied the parameter's final binding back
/// would turn a rebind into a mutation the caller never asked for — the mirror
/// image of the bug the copy-back fixes.
#[test]
fn a_callee_rebinding_its_parameter_changes_nothing_either_way() {
    let library = "pub struct P { x: integer, label: text }\n\
                   pub fn replace_all(v: vector<P>) -> integer {\n\
                   \x20   v = [];\n\
                   \x20   for i in 0..3 { v += [P { x: i * 5, label: \"r{i}\" }]; }\n\
                   \x20   len(v)\n\
                   }\n\
                   pub fn clear_it(v: vector<P>) -> integer {\n\
                   \x20   v = [];\n\
                   \x20   len(v)\n\
                   }\n\
                   pub fn make_v(n: integer) -> vector<P> {\n\
                   \x20   out: vector<P> = [];\n\
                   \x20   for i in 0..n { out += [P { x: i, label: \"e{i}\" }]; }\n\
                   \x20   out\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   v = make_v(4);\n\
                    \x20   println(\"replaced {replace_all(v)} {len(v)} {v[0].x} {v[3].label}\");\n\
                    \x20   w = make_v(4);\n\
                    \x20   println(\"cleared {clear_it(w)} {len(w)}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("rebind", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("a rebound parameter", &inproc, &placed);
    for expect in ["replaced 3 4 0 e3", "cleared 0 4"] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

/// @PLN119 arcs C+D — a worker killed with a COMPOUND value in flight, and the
/// caller's own stores surviving it.
///
/// This is the half arc D could not prove.  A crashed worker aborting the call
/// was already an error rather than a hang, but "and the caller's stores are
/// provably intact" rested on a structural argument: the worker has its own
/// address space and only values are copied in and out.
///
/// Arc B made that argument weaker, because a compound value is NOT copied
/// through a frame — it crosses in a store both processes map, and the worker
/// WRITES to the one carrying the arguments.  A worker killed mid-write leaves
/// that store in whatever state it reached, and may have resized the file out
/// from under this side's mapping.  So the failure path now maps neither arena
/// and reads nothing from them, and the dispatcher walks every compound
/// argument the caller passed with loft's own guard-before-dereference check
/// before it reports the error.
///
/// What is asserted here is what an operator sees: the program ends as a loft
/// ERROR that names the library — not a signal, not a hang, not a corruption
/// report — with the run under `LOFT_STRICT_STORES`, which turns any read of a
/// freed store during the teardown into a non-zero exit of its own.
#[test]
fn a_worker_killed_with_a_compound_in_flight_leaves_the_caller_intact() {
    let root = scratch("death_compound");
    let consumer = root.join("consumer.loft");
    std::fs::write(
        &consumer,
        "use parity::*;\n\
         fn main() {\n\
         \x20   v = make_v(2000);\n\
         \x20   println(\"before {len(v)} {v[1999].x}\");\n\
         \x20   println(\"after {grind(v, 4000000000)}\");\n\
         }\n",
    )
    .expect("write consumer");
    write_library(
        &root,
        "process",
        // The argument is a real graph — 2000 records with a text field each —
        // so the arena genuinely carries something when the worker dies.
        "pub struct P { x: integer, label: text }\n\
         pub fn make_v(n: integer) -> vector<P> {\n\
         \x20   out: vector<P> = [];\n\
         \x20   for i in 0..n { out += [P { x: i, label: \"p{i}\" }]; }\n\
         \x20   out\n\
         }\n\
         pub fn grind(v: vector<P>, n: integer) -> integer {\n\
         \x20   acc = 0;\n\
         \x20   for e in v { acc += e.x; }\n\
         \x20   for i in 0..n { acc += i; }\n\
         \x20   acc\n\
         }\n",
    );

    let child = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg("--lib")
        .arg(root.join("libs"))
        .arg(&consumer)
        .env("LOFT_TIMEOUT", "60")
        .env("LOFT_NO_NATIVE_LIBS", "1")
        .env("LOFT_STRICT_STORES", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to invoke loft");

    // Wait until the consumer is inside the long call, then kill ITS worker —
    // not the consumer. Killing too early would test a worker that never served.
    let mut pid = None;
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if let Some(p) = worker_child_of(child.id()) {
            pid = Some(p);
            break;
        }
    }
    let pid = pid.expect("the consumer never started a worker");
    // Long enough that `grind` is past its argument walk and deep in the loop.
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert!(
        Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .status()
            .expect("run kill")
            .success(),
        "could not kill the worker (pid {pid})"
    );

    let out = child.wait_with_output().expect("consumer did not finish");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("before 2000 1999"),
        "the consumer never got as far as the call: {stdout:?} / {stderr:?}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a dead worker must end the program as a loft error, not a signal \
         ({:?}); stderr: {stderr}",
        out.status
    );
    assert!(
        stderr.contains("parity") && stderr.contains("worker died"),
        "the error must name the library and what happened: {stderr}"
    );
    assert!(
        !stderr.contains("did not survive"),
        "the caller's own stores were reported broken after the worker died — \
         that is the isolation this plan claims: {stderr}"
    );
    assert!(
        !stderr.contains("[strict-store]"),
        "the failed crossing touched a freed store: {stderr}"
    );
}

/// The pid of the `--lib-worker` process `parent` started, if it has one yet.
fn worker_child_of(parent: u32) -> Option<u32> {
    let out = Command::new("pgrep")
        .arg("-P")
        .arg(parent.to_string())
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.trim().parse::<u32>().ok())
}

#[test]
fn the_gate_can_fail() {
    // A parity gate that cannot report a difference would pass forever. Give the
    // two placements DIFFERENT library sources and require the comparison to
    // notice — this is the control for every assertion above.
    let root = scratch("control");
    let consumer = root.join("consumer.loft");
    std::fs::write(
        &consumer,
        "use parity::*;\nfn main() {\n    println(\"v = {add(2, 3)}\");\n}\n",
    )
    .expect("write consumer");

    write_library(
        &root,
        "inproc",
        "pub fn add(a: integer, b: integer) -> integer { a + b }\n",
    );
    let inproc = run(&root, &consumer);
    write_library(
        &root,
        "process",
        "pub fn add(a: integer, b: integer) -> integer { a + b + 1 }\n",
    );
    let placed = run(&root, &consumer);

    assert_ne!(
        inproc.stdout, placed.stdout,
        "the gate compared two different libraries and saw no difference — it is blind"
    );
}

/// A text answer larger than the wire frame crosses intact (loft#1061).
///
/// `Value::Text` is marshalled INLINE into the request/response frame, which is a fixed
/// 1 MiB. Compound answers never had this ceiling — they are built in the return arena,
/// which grows — so a placed function whose answer was a long text hit a wall no
/// in-process call has: `git::show(sha)` on a 3.7 MB commit reached the caller as
/// `panic: return value does not fit the placement wire`, and `make view` had been
/// broken by it. An oversized text now travels in the arena too.
///
/// THE SIZE IS THE TEST. A short text takes the inline path and passes against the
/// broken build, so this asks for ~1.5 MB — comfortably past the frame — and checks the
/// BYTES, not just the length: a truncation at the old boundary would keep a plausible
/// size while losing the tail, so the head, the tail, and a slice straddling the 1 MiB
/// mark are all compared. The small answer is in the same consumer, so the inline path
/// is still exercised beside the arena one.
#[test]
fn a_text_answer_larger_than_the_frame_crosses_intact() {
    let library = "pub fn big(kib: integer) -> text {\n\
                   \x20   b = \"\";\n\
                   \x20   for _ in 0..64 { b = b + \"ABCDEFGHIJKLMNOP\"; }\n\
                   \x20   s = \"\";\n\
                   \x20   for _ in 0..kib { s = s + b; }\n\
                   \x20   s\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   small = big(1);\n\
                    \x20   println(\"small = {size(small)} {small[0 .. 4]}\");\n\
                    \x20   s = big(1536);\n\
                    \x20   println(\"size  = {size(s)}\");\n\
                    \x20   println(\"head  = {s[0 .. 16]}\");\n\
                    \x20   println(\"tail  = {s[size(s) - 16 .. size(s)]}\");\n\
                    \x20   println(\"cross = {s[1048376 .. 1048392]}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("big_text", library, consumer);
    assert_eq!(
        inproc.code, 0,
        "the in-process run must succeed: {}",
        inproc.stderr
    );
    assert_indistinguishable("a text answer past the frame size", &inproc, &placed);
    assert!(
        inproc.stdout.contains("size  = 1572864"),
        "the consumer did not build the oversized answer: {:?}",
        inproc.stdout
    );
    // `cross` starts 8 bytes into a 16-byte block (1048376 = 16 x 65523 + 8) and ends
    // past the old 1 MiB frame limit, so the correct answer is the block ROTATED — which
    // is a sharper check than an aligned slice: it pins the phase as well as the bytes.
    assert!(
        placed.stdout.contains("head  = ABCDEFGHIJKLMNOP")
            && placed.stdout.contains("tail  = ABCDEFGHIJKLMNOP")
            && placed.stdout.contains("cross = IJKLMNOPABCDEFGH"),
        "the placed answer is the wrong bytes, not merely the wrong length: {:?}",
        placed.stdout
    );
}

/// A fault inside the library reads the same way wherever it ran.
///
/// The invariant this file is for names ERROR BEHAVIOUR alongside type, effect and
/// lifetime, and nothing here covered it: every case above is a call that succeeds. A
/// `panic` in the library is the shortest thing that exercises the fault path on both
/// sides of the crossing.
#[test]
fn a_fault_inside_the_library_reads_the_same_way() {
    let library = "pub fn refuse(n: integer) -> integer {\n\
                   \x20   if n < 0 { panic(\"refusing {n}\"); }\n\
                   \x20   n * 2\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn main() {\n\
                    \x20   println(\"ok = {refuse(21)}\");\n\
                    \x20   println(\"bad = {refuse(-1)}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("fault", library, consumer);
    assert!(
        inproc.stdout.contains("ok = 42"),
        "the call before the fault must run: {:?}",
        inproc.stdout
    );
    assert_indistinguishable("a panic inside the library", &inproc, &placed);
}

/// The call chain a fault carries spans BOTH processes, and neither side can see
/// the whole of it.
///
/// `main` and `helper` run in the caller; `refuse` and `deeper` run in the
/// worker. In-process the chain is one list; placed it is two halves that have to
/// be joined in the right order, and getting the order wrong renders a chain that
/// is plausible and backwards. So this moves three axes at once against the
/// single-frame case above — the depth on the library's side, the depth on the
/// caller's, and the fault KIND, which is an `assert` here rather than a `panic`
/// and so must not pick up a `panic:` prefix on the way across.
#[test]
fn a_fault_deep_in_a_placed_library_names_every_frame() {
    let library = "fn deeper(n: integer) -> integer {\n\
                   \x20   assert(n >= 0, \"n must not be {n}\");\n\
                   \x20   n * 2\n\
                   }\n\
                   pub fn refuse(n: integer) -> integer {\n\
                   \x20   deeper(n)\n\
                   }\n";
    let consumer = "use parity::*;\n\
                    fn helper(n: integer) -> integer {\n\
                    \x20   refuse(n)\n\
                    }\n\
                    fn main() {\n\
                    \x20   println(\"ok = {helper(21)}\");\n\
                    \x20   println(\"bad = {helper(-1)}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("deepfault", library, consumer);
    assert!(
        inproc.stdout.contains("ok = 42"),
        "the call before the fault must run: {:?}",
        inproc.stdout
    );
    // Prove the gate is looking at a chain that spans both sides, rather than at
    // two runs that happen to agree on saying nothing.
    for frame in ["fn deeper()", "fn refuse()", "fn helper()", "fn main()"] {
        assert!(
            inproc.stderr.contains(frame),
            "the in-process rendering is missing {frame}: {}",
            inproc.stderr
        );
    }
    assert!(
        inproc.stderr.contains("assertion failed") && !inproc.stderr.contains("panic:"),
        "an assert must not render as a panic: {}",
        inproc.stderr
    );
    assert_indistinguishable("an assert deep inside the library", &inproc, &placed);
}
