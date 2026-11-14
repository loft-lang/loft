// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN91 — the compiled standard-library bodies the loft runner's C runtime shim includes
//! (`selfhost/crt/shim/loft_bodies.c`, written by `loft --emit-c-bodies`, `src/c/bodies.rs`).
//!
//! The committed fragment is what the C writer writes now (`make crt-bodies` regenerates it);
//! compiled into the shim, each carried body answers what the interpreter answers for its loft
//! body, read and written through the runner's stack as the bytecode's call leaves it; and the
//! shim built with `-DLOFT_CRT_NO_BODIES` carries none.
use loft::file_access as fa;
use loft::platform::process::harness_command;

const FRAGMENT: &str = "selfhost/crt/shim/loft_bodies.c";
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

/// The fragment the C writer writes now, from a program that is only `main`.
fn generated(dir: &std::path::Path) -> String {
    let stub = dir.join("stub.loft");
    fa::write(&stub, "fn main() {\n}\n").expect("write stub");
    let out = dir.join("loft_bodies.c");
    let r = harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--emit-c-bodies")
        .arg(&out)
        .arg(&stub)
        .output()
        .expect("run loft --emit-c-bodies");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    fa::read_to_string(&out).expect("read the fragment")
}

#[test]
fn the_committed_fragment_is_what_the_c_writer_writes() {
    let dir = scratch("drift");
    let now = generated(&dir);
    let committed = fa::read_to_string(FRAGMENT).expect("read the committed fragment");
    assert!(
        now == committed,
        "{FRAGMENT} is out of date — run: make crt-bodies"
    );
}

/// The C harness: a stack store, a 40-byte frame below the argument, each text as a view in
/// the argument's slot, every predicate's entry called, its answer byte and the top printed.
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
int main(int argc, char **argv) {
    int64_t io[3] = { 0, 0, 0 };
    loft_crt_stack_new(io, 3, 1 << 16);
    for (int i = 1; i < argc; i++) {
        for (size_t f = 0; f < sizeof FNS / sizeof *FNS; f++) {
            int64_t e = loft_body_find(FNS[f], (int64_t) strlen(FNS[f]));
            loft_crt_stack_str_code(io[0], 40, argv[i], 0, (int64_t) strlen(argv[i]));
            int64_t top = loft_body_call(e, io[0], 56);
            if (top < 0) { printf("-"); continue; }
            if (top != 48) { printf("(top %lld)", (long long) top); }
            printf("%d", (int) loft_crt_stack_get(0, io[0], 40));
        }
        printf("\n");
    }
    printf("unknown %lld\n", (long long) loft_body_find("t_4text_dir", 11));
    return 0;
}
"#;

/// Every text the matrix reads: ASCII cases, digits of another script, letters outside ASCII,
/// whitespace, a control character, the empty text.
const TEXTS: [&str; 12] = [
    "abc", "aBc", "ABC", "123", "a1", " ", "", "é", "ÉÀ", "٣٤", "x y", "\t",
];

/// The shim built with `defines`, linked with the harness, run over the matrix.
fn harness_output(dir: &std::path::Path, defines: &[&str]) -> String {
    let obj = dir.join("crt.o");
    let cc = harness_command("cc")
        .args(["-std=c99", "-w", "-O2", "-c"])
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
    let run = harness_command(&exe).args(TEXTS).output().expect("run");
    String::from_utf8_lossy(&run.stdout).into_owned()
}

#[test]
fn each_compiled_predicate_answers_what_its_loft_body_answers() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let dir = scratch("answers");
    // The interpreter's answers, one line per text, the predicates in the harness's order.
    let reference = dir.join("ref.loft");
    let list = TEXTS
        .iter()
        .map(|t| format!("{t:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    fa::write(
        &reference,
        format!(
            "fn b(x: boolean) -> text {{ if x {{ \"1\" }} else {{ \"0\" }} }}\n\
             fn main() {{\n  for t in [{list}] {{\n    \
             println(\"{{b(t.is_alphabetic())}}{{b(t.is_alphanumeric())}}{{b(t.is_control())}}\
             {{b(t.is_lowercase())}}{{b(t.is_numeric())}}{{b(t.is_uppercase())}}{{b(t.is_whitespace())}}\");\n  }}\n}}\n"
        ),
    )
    .expect("write the reference");
    let interp = harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg(&reference)
        .output()
        .expect("run the interpreter");
    let want = format!("{}unknown -1\n", String::from_utf8_lossy(&interp.stdout));
    assert_eq!(harness_output(&dir, &[]), want);
}

#[test]
fn a_shim_built_without_the_bodies_finds_none() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let dir = scratch("none");
    let lines = TEXTS.iter().map(|_| "-------\n").collect::<String>();
    assert_eq!(
        harness_output(&dir, &["-DLOFT_CRT_NO_BODIES"]),
        format!("{lines}unknown -1\n")
    );
}
