// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN200 B5 — the C emitter (`src/c/`), a second compiled target beside the Rust writer: a
//! scalar program written as C99, built with `cc`, prints the values the interpreter prints,
//! and both are the hand-computed ones.  Skipped (not failed) where no `cc` is on PATH.
use loft::file_access as fa;
use loft::parser::Parser;
use std::fmt::Write as _;

const SRC: &str = r#"
fn fib(n: integer) -> integer {
  if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
}
fn gcd(a: integer, b: integer) -> integer {
  x = a;
  y = b;
  while y != 0 { t = x % y; x = y; y = t; }
  x
}
fn sum_sq(n: integer) -> integer {
  s = 0;
  for i in 1..=n { s += i * i; }
  s
}
fn collatz(n: integer) -> integer {
  steps = 0;
  x = n;
  while x != 1 {
    if x % 2 == 0 { x = x / 2; } else { x = 3 * x + 1; }
    steps += 1;
  }
  steps
}
fn pairs(n: integer) -> integer {
  c = 0;
  for i in 0..n {
    if i == 7 { break; }
    for j in 0..n {
      if j <= i { continue; }
      if (i * j) % 3 == 0 { c += 1; }
    }
  }
  c
}
fn div(a: integer, b: integer) -> integer { a / b }
fn rem(a: integer, b: integer) -> integer { a % b }
fn square(x: integer) -> integer { x * x }
"#;

/// `(function, arguments, expected)` — each expected value computed by hand.
const CASES: &[(&str, &[i64], &str)] = &[
    ("fib", &[0], "0"),
    ("fib", &[1], "1"),
    ("fib", &[20], "6765"),
    ("gcd", &[1071, 462], "21"),
    ("gcd", &[17, 5], "1"),
    ("sum_sq", &[10], "385"),
    ("sum_sq", &[0], "0"),
    ("collatz", &[27], "111"),
    ("collatz", &[1], "0"),
    // i = 0..6 (the break at 7), j = i+1..9, i*j a multiple of 3: 9+3+3+6+2+2+3.
    ("pairs", &[10], "28"),
    // Integer division truncates toward zero and the remainder takes the dividend's sign.
    ("div", &[-7, 2], "-3"),
    ("rem", &[-7, 2], "-1"),
    ("square", &[3_037_000_499], "9223372030926249001"),
    // 3037000500² = 9223372037000250000 > i64::MAX: the overflow is null.
    ("square", &[3_037_000_500], "null"),
];

fn have_cc() -> bool {
    loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("loft-c-emit-{}-{name}", std::process::id()));
    fa::create_dir_all(&d).expect("scratch dir");
    d
}

fn parsed(src: &str) -> Parser {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse_str(src, "<c-emit>", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "the program must parse clean: {:?}",
        p.diagnostics.lines()
    );
    loft::scopes::check(&mut p.data, &mut p.database);
    p
}

/// Each case's printed value on the interpreter, one per line.
fn interpreted() -> Vec<String> {
    let mut prog = String::from(SRC);
    prog.push_str("fn main() {\n");
    for (f, args, _) in CASES {
        let args: Vec<String> = args.iter().map(i64::to_string).collect();
        let _ = writeln!(prog, "  println(\"{{{f}({})}}\");", args.join(", "));
    }
    prog.push_str("}\n");
    let dir = scratch("interp");
    let file = dir.join("cases.loft");
    fa::write(&file, prog).expect("write program");
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret"])
        .arg(&file)
        .output()
        .expect("run loft");
    assert!(
        out.status.success(),
        "loft: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

/// Each case's printed value from the C the emitter writes, built with `cc`.
fn compiled() -> Vec<String> {
    let p = parsed(SRC);
    let roots: Vec<u32> = CASES
        .iter()
        .map(|(f, _, _)| p.data.def_nr(&format!("n_{f}")))
        .collect();
    let mut c = loft::c::emit_unit(&p.data, &p.database, &roots)
        .unwrap_or_else(|r| panic!("the scalar subset is carried, but: {r}"));
    c.push_str("\n#include <stdio.h>\n\nstatic void show(int64_t v) {\n");
    c.push_str("    if (v == LOFT_NULL) printf(\"null\\n\"); else printf(\"%lld\\n\", (long long) v);\n}\n\n");
    c.push_str("int main(void) {\n");
    for (f, args, _) in CASES {
        let args: Vec<String> = args.iter().map(|a| format!("INT64_C({a})")).collect();
        let name = loft::c::c_name(&p.data, p.data.def_nr(&format!("n_{f}")));
        let _ = writeln!(c, "    show({name}({}));", args.join(", "));
    }
    c.push_str("    return 0;\n}\n");
    let dir = scratch("cc");
    let src = dir.join("cases.c");
    let exe = dir.join("cases");
    fa::write(&src, &c).expect("write C");
    let cc = loft::platform::process::harness_command("cc")
        .args([
            "-std=c99",
            "-O2",
            "-Wall",
            "-Werror",
            "-Wno-unused-label",
            "-Wno-unused-function",
        ])
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "cc rejected the emitted C:\n{}\n--- the C ---\n{c}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .output()
        .expect("run the C program");
    assert!(out.status.success(), "the C program failed");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn a_scalar_program_written_as_c_answers_what_the_interpreter_answers() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let want: Vec<String> = CASES.iter().map(|(_, _, e)| (*e).to_string()).collect();
    let interp = interpreted();
    assert_eq!(
        interp, want,
        "the interpreter's values are the hand-computed ones"
    );
    let c = compiled();
    assert_eq!(c, want, "the C program's values are the hand-computed ones");
}

#[test]
fn a_construct_outside_the_scalar_subset_is_refused_by_name() {
    let p = parsed(
        "struct Pt { x: integer, y: integer }\nfn up() -> integer { s: spatial<Pt[x, y]> = [Pt { x: 1, y: 2 }]; len(s) }\n",
    );
    let d = p.data.def_nr("n_up");
    let refusal = loft::c::emit_unit(&p.data, &p.database, &[d])
        .expect_err("a spatial collection is not carried yet");
    assert_eq!(refusal.function, "up");
    assert!(
        refusal.why.contains("Radix"),
        "names the construct: {refusal}"
    );
}

/// The overflow macros of `src/c/loft_cc.h` against Rust's checked arithmetic, at the
/// boundaries where a fallback can go wrong (0, ±1, ±2^31.5, ±2^32, `i64::MAX`, `i64::MIN`
/// and their neighbours), in each macro mode: a mode changes speed, never a value.
#[test]
fn the_overflow_macros_answer_as_checked_arithmetic_in_every_mode() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let mut edges: Vec<i64> = vec![0, 1, 2, 3, 3_037_000_499, 3_037_000_500, 4_294_967_296];
    for e in edges.clone() {
        edges.push(e.wrapping_neg());
        edges.push(e + 1);
        edges.push(-(e + 1));
    }
    edges.extend([
        i64::MAX,
        i64::MAX - 1,
        i64::MIN,
        i64::MIN + 1,
        i64::MAX / 2,
        i64::MIN / 2,
    ]);
    edges.sort_unstable();
    edges.dedup();
    let list: Vec<String> = edges
        .iter()
        .map(|e| {
            if *e == i64::MIN {
                "INT64_MIN".to_string()
            } else {
                format!("INT64_C({e})")
            }
        })
        .collect();
    let mut c = String::from(include_str!("../src/c/loft_cc.h"));
    let _ = write!(
        c,
        "\nstatic const int64_t edges[] = {{ {} }};\nint main(void) {{\n    size_t n = sizeof edges / sizeof edges[0];\n    for (size_t i = 0; i < n; i++) for (size_t j = 0; j < n; j++) {{\n        int64_t a = edges[i], b = edges[j], r;\n        int o = LOFT_ADD_OVF(a, b, &r); printf(\"%d %lld \", o, o ? 0LL : (long long) r);\n        o = LOFT_SUB_OVF(a, b, &r); printf(\"%d %lld \", o, o ? 0LL : (long long) r);\n        o = LOFT_MUL_OVF(a, b, &r); printf(\"%d %lld\\n\", o, o ? 0LL : (long long) r);\n    }}\n    return 0;\n}}\n",
        list.join(", ")
    );
    let mut want = String::new();
    for a in &edges {
        for b in &edges {
            for r in [a.checked_add(*b), a.checked_sub(*b), a.checked_mul(*b)] {
                match r {
                    Some(v) => {
                        let _ = write!(want, "0 {v} ");
                    }
                    None => want.push_str("1 0 "),
                }
            }
            want.pop();
            want.push('\n');
        }
    }
    let dir = scratch("ovf");
    let src = dir.join("ovf.c");
    fa::write(&src, &c).expect("write C");
    for (mode, flags) in [
        ("strict", &["-DLOFT_STRICT", "-pedantic", "-Werror"][..]),
        ("default", &[][..]),
        ("verify", &["-DLOFT_VERIFY"][..]),
    ] {
        let exe = dir.join(format!("ovf-{mode}"));
        let cc = loft::platform::process::harness_command("cc")
            .args(["-std=c99", "-O2"])
            .args(flags)
            .arg(&src)
            .arg("-o")
            .arg(&exe)
            .output()
            .expect("run cc");
        assert!(
            cc.status.success(),
            "{mode}: cc rejected the overflow macros:\n{}",
            String::from_utf8_lossy(&cc.stderr)
        );
        let out = loft::platform::process::harness_command(&exe)
            .output()
            .expect("run the overflow program");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            want,
            "{mode}: the overflow macros disagree with checked arithmetic"
        );
    }
}

/// The keyed collections as C (`src/c/loft_keyed.h`): a hash keeps the latest record per key and
/// walks in key order; a descending sorted range runs in the declared order and stops at an
/// inclusive bound; an index walks a range reversed; a linked group files every record in each
/// member, and a key collision in the hash leaves the vector member holding both.  Each value is
/// computed by hand, and the interpreter and the C must both answer it.
#[test]
fn keyed_collections_written_as_c_answer_the_hand_computed_values() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"
struct P { k: integer, v: text }
struct F { k: integer, t: text }
struct G { es: vector<F>, byk: hash<F[k]> }
fn main() {
  h: hash<P[v]> = [P { k: 1, v: "b" }, P { k: 2, v: "a" }, P { k: 3, v: "b" }];
  t = "";
  for p in h { t += "{p.v}{p.k},"; }
  println("{len(h)} {t}");
  s: sorted<P[-k]> = [P { k: 1, v: "x" }, P { k: 5, v: "y" }, P { k: 3, v: "z" }];
  u = "";
  for p in s[4..=1] { u += "{p.k},"; }
  println(u);
  ix: index<P[k]> = [P { k: 10, v: "p" }, P { k: 20, v: "q" }, P { k: 30, v: "r" }];
  w = "";
  for p in rev(ix[15..]) { w += "{p.v},"; }
  println("{w} {ix[20].v}");
  g = G {};
  g.es += [F { k: 1, t: "a" }, F { k: 2, t: "b" }, F { k: 1, t: "c" }];
  println("{len(g.es)} {len(g.byk)} {g.byk[1].t}");
}
"#;
    // h: "b" twice, the later (k 3) wins; key order a, b.  s walks 5, 3, 1 (descending): from 4
    // (the first key at or past it is 3) to 1 inclusive.  ix from 15: 20, 30, reversed.  The
    // group: the vector member keeps all three records, the hash member one per key, k 1 the
    // later "c".
    let want = "2 a2,b3,\n3,1,\nr,q, q\n3 2 c\n";
    let dir = scratch("keyed");
    let file = dir.join("keyed.loft");
    fa::write(&file, src).expect("write program");
    let loft = env!("CARGO_BIN_EXE_loft");
    let interp = loft::platform::process::harness_command(loft)
        .arg("--interpret")
        .arg(&file)
        .output()
        .expect("run loft");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout),
        want,
        "the interpreter"
    );
    let c = dir.join("keyed.c");
    let emit = loft::platform::process::harness_command(loft)
        .arg("--emit-c")
        .arg(&c)
        .arg(&file)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "the keyed program is carried: {}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let exe = dir.join("keyed");
    let cc = loft::platform::process::harness_command("cc")
        .args(["-std=c99", "-O1", "-DLOFT_STRICT", "-pedantic", "-Werror"])
        .arg(&c)
        .arg("-o")
        .arg(&exe)
        .arg("-lm")
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "cc rejected the keyed C:\n{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .output()
        .expect("run the keyed C program");
    assert_eq!(String::from_utf8_lossy(&out.stdout), want, "the C program");
}

/// Files as C (`src/c/loft_io.h`): a record, a sized integer and a text written in their binary
/// form and read back in order through a second handle, a text file written and read as lines,
/// and a deleted path no longer existing.  Each value is computed by hand; the C is built twice
/// — against POSIX and, with `-DLOFT_NO_POSIX`, against ISO C99 alone — and both must answer it.
#[test]
fn files_written_as_c_answer_the_hand_computed_values() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"
struct P { x: integer, y: i32 }
fn main() {
  f = file("c_emit_files.bin");
  f#format = LittleEndian;
  f += P { x: 5, y: -6 };
  f += (513 as u16);
  f += "hi";
  g = file("c_emit_files.bin");
  g#format = LittleEndian;
  q = g#read as P;
  u = g#read as u16;
  t = g#read(2) as text;
  println("{q.x} {q.y} {u} {t} {g.size}");
  h = file("c_emit_files.txt");
  h.write("a\nb\n");
  println("{file(\"c_emit_files.txt\").lines()}");
  delete("c_emit_files.bin");
  delete("c_emit_files.txt");
  println("{exists(\"c_emit_files.bin\")}");
}
"#;
    // The record is 8 + 4 bytes, the u16 2, the text 2: 16 bytes, which `file()` reports.
    let want = "5 -6 513 hi 16\n[\"a\",\"b\"]\nfalse\n";
    let loft = env!("CARGO_BIN_EXE_loft");
    for (mode, flags) in [
        ("posix", &["-DLOFT_STRICT"][..]),
        ("iso", &["-DLOFT_STRICT", "-DLOFT_NO_POSIX"][..]),
    ] {
        let dir = scratch(&format!("files-{mode}"));
        let file = dir.join("files.loft");
        fa::write(&file, src).expect("write program");
        if mode == "posix" {
            let interp = loft::platform::process::harness_command(loft)
                .arg("--interpret")
                .arg(&file)
                .output()
                .expect("run loft");
            assert_eq!(
                String::from_utf8_lossy(&interp.stdout),
                want,
                "the interpreter"
            );
        }
        let c = dir.join("files.c");
        let emit = loft::platform::process::harness_command(loft)
            .arg("--emit-c")
            .arg(&c)
            .arg(&file)
            .output()
            .expect("run loft --emit-c");
        assert!(
            emit.status.success(),
            "the file program is carried: {}",
            String::from_utf8_lossy(&emit.stderr)
        );
        let exe = dir.join("files");
        let cc = loft::platform::process::harness_command("cc")
            .args(["-std=c99", "-O1", "-pedantic", "-Werror"])
            .args(flags)
            .arg(&c)
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .output()
            .expect("run cc");
        assert!(
            cc.status.success(),
            "{mode}: cc rejected the file C:\n{}",
            String::from_utf8_lossy(&cc.stderr)
        );
        let out = loft::platform::process::harness_command(&exe)
            .current_dir(&dir)
            .output()
            .expect("run the file C program");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            want,
            "{mode}: the C program"
        );
    }
}

/// Generators as C (`src/c/body/generator.rs`): a generator of texts over a text parameter ended
/// early by `break`, one of integers through nested loops with a `continue`, and one stepped by
/// hand and abandoned before its end.  Each value is computed by hand; the interpreter and the C
/// must both answer it, and the C must stay clean under AddressSanitizer where `cc` offers it.
#[test]
fn generators_written_as_c_answer_the_hand_computed_values() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"
fn words(prefix: text, n: integer) -> iterator<text> {
  for i in 0..n {
    if i == 3 { break; }
    yield "{prefix}-{i}";
  }
}
fn grid(w: integer, h: integer) -> iterator<integer> {
  for y in 0..h { for x in 0..w { if x == y { continue; } yield y * 10 + x; } }
}
fn main() {
  t = "";
  for w in words("ab", 9) { t += w + ","; }
  println(t);
  s = 0;
  for v in grid(3, 3) { s += v; }
  println("{s}");
  it = words("x", 5);
  a = next(it);
  b = next(it);
  println("{a} {b}");
}
"#;
    // words stops at i == 3; grid skips the diagonal: 1+2+10+12+20+21 = 66; `it` is left after
    // two values, its frame released at the end of `main`.
    let want = "ab-0,ab-1,ab-2,\n66\nx-0 x-1\n";
    let dir = scratch("generators");
    let file = dir.join("generators.loft");
    fa::write(&file, src).expect("write program");
    let loft = env!("CARGO_BIN_EXE_loft");
    let interp = loft::platform::process::harness_command(loft)
        .arg("--interpret")
        .arg(&file)
        .output()
        .expect("run loft");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout),
        want,
        "the interpreter"
    );
    let c = dir.join("generators.c");
    let emit = loft::platform::process::harness_command(loft)
        .arg("--emit-c")
        .arg(&c)
        .arg(&file)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "the generators are carried: {}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let exe = dir.join("generators");
    let cc = loft::platform::process::harness_command("cc")
        .args(["-std=c99", "-O1", "-DLOFT_STRICT", "-pedantic", "-Werror"])
        .arg(&c)
        .arg("-o")
        .arg(&exe)
        .arg("-lm")
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "cc rejected the generator C:\n{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .output()
        .expect("run the generator C program");
    assert_eq!(String::from_utf8_lossy(&out.stdout), want, "the C program");
}

/// `par` as C (`src/c/body/par.rs`): a worker over integer rows, one over record rows that
/// returns a text, and one with an extra argument.  The C runs the rows in order; each result
/// is the one a thread computes, so the hand-computed values hold on both.
#[test]
fn par_loops_written_as_c_answer_the_hand_computed_values() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"
struct It { name: text, n: integer }
fn sq(x: integer) -> integer { x * x }
fn label(it: const It) -> text { "{it.name}{it.n}" }
fn scaled(x: integer, k: integer) -> integer { x * k }
fn main() {
  v = [1, 2, 3, 4];
  s = 0;
  for x in v par(r = sq(x), 2) { s += r; }
  items = [It { name: "a", n: 1 }, It { name: "b", n: 2 }];
  t = "";
  for it in items par(l = label(it), 2) { t += l + ","; }
  k = 10;
  u = 0;
  for x in v par(m = scaled(x, k), 3) { u += m; }
  println("{s} {t} {u}");
}
"#;
    // 1+4+9+16 = 30; the labels in row order; (1+2+3+4)*10 = 100.
    let want = "30 a1,b2, 100\n";
    let dir = scratch("par");
    let file = dir.join("par.loft");
    fa::write(&file, src).expect("write program");
    let loft = env!("CARGO_BIN_EXE_loft");
    let interp = loft::platform::process::harness_command(loft)
        .arg("--interpret")
        .arg(&file)
        .output()
        .expect("run loft");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout),
        want,
        "the interpreter"
    );
    let c = dir.join("par.c");
    let emit = loft::platform::process::harness_command(loft)
        .arg("--emit-c")
        .arg(&c)
        .arg(&file)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "the par loops are carried: {}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let exe = dir.join("par");
    let cc = loft::platform::process::harness_command("cc")
        .args(["-std=c99", "-O1", "-DLOFT_STRICT", "-pedantic", "-Werror"])
        .arg(&c)
        .arg("-o")
        .arg(&exe)
        .arg("-lm")
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "cc rejected the par C:\n{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .output()
        .expect("run the par C program");
    assert_eq!(String::from_utf8_lossy(&out.stdout), want, "the C program");
}
