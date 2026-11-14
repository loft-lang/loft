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
//!
//! A body that answers a TEXT appends it to a destination, as the Rust bridge does: the
//! destination form to the text its reference names (what was there stays), the plain form to
//! the call's own work cell, whose view it pushes.
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

/// What both harnesses read their texts with: the fields of a file, each ended by the byte
/// 0x1F.  A command line carries no text outside the code page on Windows, and a text of the
/// matrix may be empty or a tab, so neither arguments nor lines can hold them.
const FIELDS: &str = r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static char **loft_fields(const char *path, int *n) {
    FILE *f = fopen(path, "rb");
    *n = 0;
    if (!f) return 0;
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *all = (char *) malloc((size_t) size + 1);
    if (fread(all, 1, (size_t) size, f) != (size_t) size) size = 0;
    fclose(f);
    all[size] = 0;
    int count = 0;
    for (long i = 0; i < size; i++) count += all[i] == 0x1F;
    char **out = (char **) malloc(sizeof(char *) * (size_t) (count + 1));
    char *at = all;
    for (long i = 0; i < size; i++) {
        if (all[i] == 0x1F) {
            all[i] = 0;
            out[(*n)++] = at;
            at = all + i + 1;
        }
    }
    return out;
}
"#;

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
        int n = 0;
        char **t = argc == 2 ? loft_fields(argv[1], &n) : 0;
        for (int i = 0; i < n; i++) line(t[i], (int64_t) strlen(t[i]));
    }
    printf("unknown %lld\n", (long long) loft_body_find("t_4File_lines", 13));
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
    harness_of(dir, defines, HARNESS)
}

fn harness_of(dir: &std::path::Path, defines: &[&str], harness: &str) -> std::path::PathBuf {
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
    fa::write(&src, format!("{FIELDS}{harness}")).expect("write the harness");
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

/// A passed test leaves nothing behind (the scratch is a RAM disk on some boxes).
fn done(dir: &std::path::Path) {
    let _ = fa::remove_dir_all(dir);
}

/// The harness run on `args`, its output with the host's line ends as `\n`.  The texts go to
/// it in a file (`FIELDS`), never on the command line; `sweep` alone is a word of its own.
fn run(exe: &std::path::Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let mut c = harness_command(exe);
    if args == ["sweep"] {
        c.args(args);
    } else {
        let fields = exe
            .parent()
            .expect("the harness has a directory")
            .join("fields.bin");
        let bytes: Vec<u8> = args
            .iter()
            .flat_map(|a| a.bytes().chain(std::iter::once(0x1F)))
            .collect();
        fa::write(&fields, bytes).expect("write the fields");
        c.arg(&fields);
    }
    for (k, v) in env {
        c.env(k, v);
    }
    String::from_utf8_lossy(&c.output().expect("run the harness").stdout).replace("\r\n", "\n")
}

/// The interpreter's answers for `main`'s body.
fn interpreted(dir: &std::path::Path, main: &str) -> String {
    format!(
        "{}unknown -1\n",
        interpret(dir, &format!("{REFERENCE}fn main() {{\n{main}}}\n"))
    )
}

/// What the interpreter prints for `program`.
fn interpret(dir: &std::path::Path, program: &str) -> String {
    let p = dir.join("ref.loft");
    fa::write(&p, program).expect("write the reference");
    let r = harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg(&p)
        .output()
        .expect("run the interpreter");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n")
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
    done(&dir);
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
    done(&dir);
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
    done(&dir);
    let dir = scratch("out");
    let exe = harness(&dir, &["-DLOFT_CRT_NO_BODIES"]);
    assert_eq!(run(&exe, &TEXTS, &[]), none);
    done(&dir);
}

/// The text answers' harness.  The frame: a destination text at 0 holding `<`, two work cells
/// at 24 and 48, the arguments from 80.  Per row `a b from to`, each function's destination
/// form (the destination read back) and, where the call has a work cell, its plain form (the
/// pushed view read back); `!` for a wrong top, `-` where no entry answers.
const TEXT_HARNESS: &str = r#"#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int64_t loft_crt_stack_new(int64_t *io, int64_t n, int64_t bytes);
void loft_crt_stack_str_code(int64_t store, int64_t at, const char *code, int64_t from, int64_t len);
void loft_crt_stack_put(int64_t which, int64_t store, int64_t at, int64_t v, int64_t is_null);
int64_t loft_crt_slot_text(char *out, int64_t cap, int64_t store, int64_t at, int64_t arg);
void loft_crt_slot_text_init(int64_t store, int64_t at);
void loft_crt_set_slot_text(int64_t store, int64_t at, const char *val, int64_t bytes);
void loft_crt_slot_text_clear(int64_t store, int64_t at);
void loft_crt_set_place_ref(int64_t store, int64_t rec, int64_t pos, int64_t r_store, int64_t r_rec, int64_t r_pos);
int64_t loft_body_find(const char *name, int64_t len);
int64_t loft_body_call(int64_t entry, int64_t stk, int64_t sp);
int64_t loft_body_call_dest(int64_t entry, int64_t stk, int64_t sp, int64_t s, int64_t r, int64_t p);
static int64_t stk;
static const char *a, *b;
static int64_t from, to;
/* The arguments of `fn` laid from 80; the stack top. */
static int64_t lay(int fn) {
    int64_t at = 80;
    loft_crt_stack_str_code(stk, at, a, 0, (int64_t) strlen(a)); at += 16;
    if (fn == 2) {
        loft_crt_stack_str_code(stk, at, b, 0, (int64_t) strlen(b)); at += 16;
        loft_crt_set_place_ref(stk, 1, 8 + at, stk, 1, 8 + 24); at += 16;
        loft_crt_set_place_ref(stk, 1, 8 + at, stk, 1, 8 + 48); at += 16;
    }
    if (fn == 3) {
        loft_crt_stack_put(6, stk, at, from, 0); at += 8;
        loft_crt_stack_put(6, stk, at, to, 0); at += 8;
        loft_crt_set_place_ref(stk, 1, 8 + at, stk, 1, 8 + 24); at += 16;
    }
    loft_crt_slot_text_clear(stk, 24);
    loft_crt_slot_text_clear(stk, 48);
    return at;
}
static void show(int64_t at, int64_t arg) {
    char out[4096];
    int64_t n = loft_crt_slot_text(out, sizeof out, stk, at, arg);
    if (n < 0) printf("(null)"); else printf("%.*s", (int) n, out);
}
int main(int argc, char **argv) {
    static const char *FNS[] = { "t_4text_dir", "loft_shared_t_4text_basename", "t_4text_resolve", "t_4text_char_slice" };
    int64_t io[3] = { 0, 0, 0 };
    loft_crt_stack_new(io, 3, 1 << 16);
    stk = io[0];
    loft_crt_slot_text_init(stk, 0);
    loft_crt_slot_text_init(stk, 24);
    loft_crt_slot_text_init(stk, 48);
    int n = 0;
    char **t = argc == 2 ? loft_fields(argv[1], &n) : 0;
    for (int i = 0; i + 3 < n; i += 4) {
        a = t[i]; b = t[i + 1]; from = atoll(t[i + 2]); to = atoll(t[i + 3]);
        for (int fn = 0; fn < 4; fn++) {
            int64_t e = loft_body_find(FNS[fn], (int64_t) strlen(FNS[fn]));
            loft_crt_set_slot_text(stk, 0, "<", 1);
            int64_t top = loft_body_call_dest(e, stk, lay(fn), stk, 1, 8);
            if (top == 80) show(0, 0); else printf(top < 0 ? "-" : "!");
            printf("|");
            top = loft_body_call(e, stk, lay(fn));
            if (top == 96) show(80, 1); else printf(top < 0 ? "-" : "!");
            printf("|");
        }
        printf("\n");
    }
    /* A predicate has no destination form. */
    int64_t p = loft_body_find("t_4text_is_numeric", 18);
    loft_crt_stack_str_code(stk, 80, "12", 0, 2);
    printf("predicate %lld\n", (long long) loft_body_call_dest(p, stk, 96, stk, 1, 8));
    return 0;
}
"#;

/// Paths, targets and slice bounds: nested, bare, rooted, trailing slash, empty; `./` and `../`
/// walks past the root, an absolute target; bounds from the end, past the end, crossed, and a
/// text whose characters are wider than a byte.
const PATHS: [&str; 7] = ["doc/claude/x.md", "x.md", "/foo/", "", "a/b/c", "/", "é/ö"];
const TARGETS: [&str; 7] = [
    "../y.md",
    "./z",
    "../../q",
    "/abs",
    "w",
    "",
    "./../../../../up",
];
const BOUNDS: [(i64, i64); 5] = [(1, 4), (-3, -1), (0, 100), (5, 2), (0, 0)];

fn text_rows() -> Vec<(&'static str, &'static str, i64, i64)> {
    let mut rows = Vec::new();
    for (i, a) in PATHS.iter().enumerate() {
        for (j, b) in TARGETS.iter().enumerate() {
            let (f, t) = BOUNDS[(i + j) % BOUNDS.len()];
            rows.push((*a, *b, f, t));
        }
    }
    rows
}

#[test]
fn each_compiled_text_answer_is_appended_as_its_loft_body_answers() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let dir = scratch("texts");
    let exe = harness_of(&dir, &[], TEXT_HARNESS);
    let rows = text_rows();
    let mut program = String::from(
        "fn row(a: text, b: text, f: integer, t: integer) {\n  \
         println(\"<{a.dir()}|-|<{a.basename()}|-|<{a.resolve(b)}|{a.resolve(b)}|\
         <{a.char_slice(f, t)}|{a.char_slice(f, t)}|\");\n}\nfn main() {\n",
    );
    let mut args = Vec::new();
    for (a, b, f, t) in &rows {
        program += &format!("  row({a:?}, {b:?}, {f}, {t});\n");
        args.extend([
            (*a).to_string(),
            (*b).to_string(),
            f.to_string(),
            t.to_string(),
        ]);
    }
    program += "  println(\"predicate -1\");\n}\n";
    let want = interpret(&dir, &program);
    assert_eq!(want.lines().count(), rows.len() + 1);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    assert_eq!(run(&exe, &args, &[]), want);
    // Switched off, no entry answers either form.
    let off = run(&exe, &args, &[("LOFT_NO_COMPILED_BODIES", "1")]);
    assert_eq!(off.lines().next(), Some("-|-|-|-|-|-|-|-|"));
    done(&dir);
}
