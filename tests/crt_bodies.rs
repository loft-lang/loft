// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN91 — the compiled standard-library bodies the loft runner's C runtime shim includes
//! (`loft --emit-c-bodies`, `src/c/bodies.rs`; written per build by `c_shim::put_bodies`).
//!
//! Compiled into the shim, each carried body answers what the interpreter of the SAME binary
//! answers for its loft body — over a matrix of texts, and for every 7th code point of Unicode
//! (the predicates' tables are the building binary's, which is why the fragment is never
//! committed) — read and written through the runner's stack as the bytecode's call leaves it;
//! `LOFT_NO_COMPILED_BODIES=1` and a shim built with `-DLOFT_CRT_NO_BODIES` carry none.
use loft::file_access as fa;
use loft::platform::process::harness_command;

const SHIM: &str = "selfhost/crt/shim/crt.c";

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("loft-crt-bodies-{}-{name}", std::process::id()));
    fa::create_dir_all(&d).expect("scratch dir");
    d
}

fn have_cc() -> bool {
    harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The fragment this binary writes, into `dir` (where the shim's `<loft_bodies.c>` finds it).
fn generate(dir: &std::path::Path) {
    let stub = dir.join("stub.loft");
    fa::write(&stub, "fn main() {\n}\n").expect("write stub");
    let r = harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--emit-c-bodies")
        .arg(dir.join("loft_bodies.c"))
        .arg(&stub)
        .output()
        .expect("run loft --emit-c-bodies");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
}

/// The harness: a stack store, a 40-byte frame below the argument, each text as a view in the
/// argument's slot, every predicate's entry called, its answer byte (or `-` where none answers,
/// or the wrong top) printed.  The texts are its arguments, or with `sweep`, every 7th code
/// point outside the surrogates as a one-character text.
const HARNESS: &str = r#"#include <stdint.h>
#include <stdio.h>
#include <string.h>
int64_t loft_crt_stack_new(int64_t *io, int64_t n, int64_t bytes);
void loft_crt_stack_str_code(int64_t store, int64_t at, const char *code, int64_t from, int64_t len);
int64_t loft_crt_stack_get(int64_t which, int64_t store, int64_t at);
int64_t loft_body_find(const char *name, int64_t len);
int64_t loft_body_call(int64_t entry, int64_t stk, int64_t sp);
static const char *FNS[] = { "t_4text_is_alphabetic", "t_4text_is_alphanumeric",
    "t_4text_is_control", "t_4text_is_lowercase", "loft_shared_t_4text_is_numeric",
    "t_4text_is_uppercase", "t_4text_is_whitespace" };
static int64_t stk;
static void line(const char *t, int64_t len) {
    for (size_t f = 0; f < sizeof FNS / sizeof *FNS; f++) {
        int64_t e = loft_body_find(FNS[f], (int64_t) strlen(FNS[f]));
        loft_crt_stack_str_code(stk, 40, t, 0, len);
        int64_t top = loft_body_call(e, stk, 56);
        if (top != 48) { printf("-"); continue; }
        printf("%d", (int) loft_crt_stack_get(0, stk, 40));
    }
    printf("\n");
}
static int utf8(uint32_t c, char *o) {
    if (c < 0x80) { o[0] = (char) c; return 1; }
    if (c < 0x800) { o[0] = (char) (0xC0 | (c >> 6)); o[1] = (char) (0x80 | (c & 63)); return 2; }
    if (c < 0x10000) { o[0] = (char) (0xE0 | (c >> 12)); o[1] = (char) (0x80 | ((c >> 6) & 63)); o[2] = (char) (0x80 | (c & 63)); return 3; }
    o[0] = (char) (0xF0 | (c >> 18)); o[1] = (char) (0x80 | ((c >> 12) & 63)); o[2] = (char) (0x80 | ((c >> 6) & 63)); o[3] = (char) (0x80 | (c & 63)); return 4;
}
int main(int argc, char **argv) {
    int64_t io[3] = { 0, 0, 0 };
    loft_crt_stack_new(io, 3, 1 << 16);
    stk = io[0];
    if (argc == 2 && strcmp(argv[1], "sweep") == 0) {
        char b[4];
        for (uint32_t c = 1; c < 1114112; c += 7) {
            if (c >= 55296 && c <= 57343) continue;
            line(b, utf8(c, b));
        }
    } else {
        for (int i = 1; i < argc; i++) line(argv[i], (int64_t) strlen(argv[i]));
    }
    printf("unknown %lld\n", (long long) loft_body_find("t_4text_dir", 11));
    return 0;
}
"#;

/// The predicates' answers on the interpreter for each text of `texts` (a loft expression).
const REFERENCE: &str = "fn b(x: boolean) -> text { if x { \"1\" } else { \"0\" } }\n\
     fn row(t: text) {\n  \
     println(\"{b(t.is_alphabetic())}{b(t.is_alphanumeric())}{b(t.is_control())}\
     {b(t.is_lowercase())}{b(t.is_numeric())}{b(t.is_uppercase())}{b(t.is_whitespace())}\");\n}\n";

/// Every text of the matrix: ASCII cases, digits of another script, letters outside ASCII,
/// whitespace, a control character, the empty text.
const TEXTS: [&str; 12] = [
    "abc", "aBc", "ABC", "123", "a1", " ", "", "é", "ÉÀ", "٣٤", "x y", "\t",
];

/// The shim built with this binary's fragment (or `defines`), linked with the harness.
fn harness(dir: &std::path::Path, defines: &[&str]) -> std::path::PathBuf {
    generate(dir);
    let obj = dir.join("crt.o");
    let cc = harness_command("cc")
        .args(["-std=c99", "-w", "-O2", "-c", "-I"])
        .arg(dir)
        .args(defines)
        .arg(SHIM)
        .arg("-o")
        .arg(&obj)
        .output()
        .expect("run cc on the shim");
    assert!(
        cc.status.success(),
        "{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let src = dir.join("harness.c");
    fa::write(&src, HARNESS).expect("write the harness");
    let exe = dir.join("harness");
    let link = harness_command("cc")
        .args(["-std=c99", "-w", "-O2"])
        .arg(&src)
        .arg(&obj)
        .arg("-o")
        .arg(&exe)
        .args(["-lm", "-lpthread"])
        .output()
        .expect("link the harness");
    assert!(
        link.status.success(),
        "{}",
        String::from_utf8_lossy(&link.stderr)
    );
    exe
}

fn run(exe: &std::path::Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let mut c = harness_command(exe);
    c.args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    String::from_utf8_lossy(&c.output().expect("run the harness").stdout).into_owned()
}

/// The interpreter's answers for `main`'s body.
fn interpreted(dir: &std::path::Path, main: &str) -> String {
    let p = dir.join("ref.loft");
    fa::write(&p, format!("{REFERENCE}fn main() {{\n{main}}}\n")).expect("write the reference");
    let r = harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg(&p)
        .output()
        .expect("run the interpreter");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    format!("{}unknown -1\n", String::from_utf8_lossy(&r.stdout))
}

#[test]
fn each_compiled_predicate_answers_what_its_loft_body_answers() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let dir = scratch("answers");
    let exe = harness(&dir, &[]);
    let list = TEXTS
        .iter()
        .map(|t| format!("{t:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let want = interpreted(&dir, &format!("  for t in [{list}] {{ row(t); }}\n"));
    assert_eq!(run(&exe, &TEXTS, &[]), want);
}

#[test]
fn the_compiled_predicates_agree_with_the_interpreter_over_unicode() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let dir = scratch("unicode");
    let exe = harness(&dir, &[]);
    let want = interpreted(
        &dir,
        "  cp = 1;\n  while cp < 1114112 {\n    \
         if cp < 55296 || cp > 57343 { row(\"{cp as character}\"); }\n    cp += 7;\n  }\n",
    );
    let got = run(&exe, &["sweep"], &[]);
    if got != want {
        let (g, w): (Vec<&str>, Vec<&str>) = (got.lines().collect(), want.lines().collect());
        let at = g
            .iter()
            .zip(&w)
            .position(|(a, b)| a != b)
            .unwrap_or(g.len().min(w.len()));
        panic!(
            "code point row {at} (1 + 7 x {at}, surrogates skipped): compiled {:?}, interpreted {:?}",
            g.get(at),
            w.get(at)
        );
    }
}

#[test]
fn no_body_answers_when_switched_off_or_left_out() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let none = format!(
        "{}unknown -1\n",
        TEXTS.iter().map(|_| "-------\n").collect::<String>()
    );
    let dir = scratch("off");
    let exe = harness(&dir, &[]);
    assert_eq!(run(&exe, &TEXTS, &[("LOFT_NO_COMPILED_BODIES", "1")]), none);
    let dir = scratch("out");
    let exe = harness(&dir, &["-DLOFT_CRT_NO_BODIES"]);
    assert_eq!(run(&exe, &TEXTS, &[]), none);
}
