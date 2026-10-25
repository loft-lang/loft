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
    let p = parsed("fn up() -> integer { v = \"[1,2]\" as vector<integer>; len(v) }\n");
    let d = p.data.def_nr("n_up");
    let refusal = loft::c::emit_unit(&p.data, &p.database, &[d])
        .expect_err("text parsed into a vector is not carried yet");
    assert_eq!(refusal.function, "up");
    assert!(
        refusal.why.contains("OpCastVectorFromText"),
        "names the construct: {refusal}"
    );
}

/// `file_map` is the one producer of a foreign store (@PLN174): a unit that reaches it compiles
/// `loft_at` with the foreign record's test, and no other unit pays that test.  The values are
/// the corpus's (`tests/scripts/174-foreign-*.loft`, through `scripts/c_sweep.py`).
#[test]
fn only_a_unit_that_maps_a_file_compiles_the_foreign_record() {
    let p = parsed(
        "fn mapped(p: text) -> integer { m = file_map(p) ?? []; len(m) }\n\
         fn copied(p: text) -> integer { m = read_bytes(p) ?? []; len(m) }\n",
    );
    let unit = |f: &str| {
        loft::c::emit_unit(&p.data, &p.database, &[p.data.def_nr(&format!("n_{f}"))])
            .unwrap_or_else(|r| panic!("{f} is carried: {r}"))
    };
    assert!(
        unit("mapped").starts_with("#define LOFT_FOREIGN 1\n"),
        "a unit that maps a file serves the foreign record"
    );
    assert!(
        !unit("copied").contains("#define LOFT_FOREIGN 1"),
        "a unit that maps nothing compiles loft_at without the foreign test"
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

/// The C store allocator (`src/c/loft_store.h`) is a PORT of `Store`'s, not one of its own
/// (@PLN200 B5, owner): one seeded sequence of claims, deletes, resizes and referenced-record
/// marks runs through `Store` and through the C, and every position either answers, then the
/// block chain the run leaves (each block's header, and a free block's footer), must be the
/// same.  A sequence long and varied enough to reach each path: the wilderness bump, the
/// parked lists, the lazy phase and its sweep, best fit with carving, the backward merge, the
/// growth, an in-place and a moving resize, and a tombstone.  The stores are DEFINED (zeroed
/// when made and grown) and the test writes no payload, so every byte is the allocator's and
/// the whole buffer is compared — which a program's store never allows, since a claim promises
/// no value (`@FR-H-Claim`); a program is compared by walking its data instead.  Falsified: a
/// parked-size bound, the lazy floor, the in-place resize's slack, the tombstone and carving
/// switched off each fail it.
#[test]
fn the_c_store_allocator_places_every_record_where_store_does() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    // A fixed LCG, so the sequence is the same on every run and every machine.
    let mut seed: u64 = 0x5eed_1234_abcd_0001;
    let mut next = |n: u64| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % n
    };
    // DEFINED stores (zeroed when made and grown), so every byte is the allocator's to write.
    let mut store = loft::store::Store::new_in_use_defined(100);
    let mut live: Vec<u32> = Vec::new();
    let (mut script, mut want) = (String::new(), String::new());
    for _ in 0..6000 {
        let pick = next(100);
        if pick < 50 || live.is_empty() {
            let size = match next(20) {
                0..=11 => 1 + next(8),
                12..=16 => 9 + next(12),
                17..=18 => 40 + next(40),
                _ => 150 + next(250),
            } as u32;
            let pos = store.claim(size);
            live.push(pos);
            let _ = writeln!(script, "C {size}");
            let _ = writeln!(want, "{pos}");
        } else if pick < 80 {
            let rec = live.swap_remove(next(live.len() as u64) as usize);
            store.delete(rec);
            let _ = writeln!(script, "D {rec}");
        } else if pick < 95 {
            let i = next(live.len() as u64) as usize;
            let rec = live[i];
            let words = store.read::<i32>(rec, 0) as u32;
            let size = words + 1 + next(u64::from(words)) as u32;
            let pos = store.resize(rec, size);
            live[i] = pos;
            let _ = writeln!(script, "R {rec} {size}");
            let _ = writeln!(want, "{pos}");
        } else {
            let rec = live[next(live.len() as u64) as usize];
            store.note_referenced(rec);
            let _ = writeln!(script, "N {rec}");
        }
    }
    let _ = writeln!(want, "size {}", store.capacity_words());
    // FNV-1a over every byte: the free tree's links and colours, the parked lists' links, the
    // footers — what the chain below does not show, and what a tree that places every record
    // alike can still disagree on.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for w in 0..store.capacity_words() {
        for b in store.read::<u64>(w, 0).to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    let _ = writeln!(want, "bytes {h:016x}");
    let mut pos = 1u32;
    while pos < store.capacity_words() {
        let h = store.read::<i32>(pos, 0);
        let words = h.unsigned_abs();
        if h < 0 {
            let footer = store.read::<i32>(pos + words - 1, 4);
            let _ = writeln!(want, "{pos} {h} {footer}");
        } else {
            let _ = writeln!(want, "{pos} {h}");
        }
        pos += words;
    }
    let mut c = String::from(
        "#include <stdint.h>\n#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n",
    );
    c.push_str(include_str!("../src/c/loft_store.h"));
    c.push_str(
        r#"
int main(int argc, char **argv) {
    loft_store_t s;
    memset(&s, 0, sizeof s);
    s.size = 100;
    s.buf = (uint8_t *) calloc(s.size, 8);
    s.defined = 1;
    loft_st_init(&s);
    FILE *f = fopen(argv[argc - 1], "r");
    char op;
    unsigned a, b;
    while (fscanf(f, " %c", &op) == 1) {
        if (op == 'C') { if (fscanf(f, "%u", &a) != 1) return 2; printf("%u\n", loft_st_claim(&s, a)); }
        else if (op == 'D') { if (fscanf(f, "%u", &a) != 1) return 2; loft_st_delete(&s, a); }
        else if (op == 'R') { if (fscanf(f, "%u %u", &a, &b) != 2) return 2; printf("%u\n", loft_st_resize(&s, a, b)); }
        else if (op == 'N') { if (fscanf(f, "%u", &a) != 1) return 2; loft_st_note_referenced(&s, a); }
    }
    printf("size %u\n", s.size);
    uint64_t h = UINT64_C(0xcbf29ce484222325);
    for (size_t i = 0; i < (size_t) s.size * 8; i++) h = (h ^ s.buf[i]) * UINT64_C(0x100000001b3);
    printf("bytes %016llx\n", (unsigned long long) h);
    for (uint32_t pos = 1; pos < s.size;) {
        int32_t h = loft_st_i32(&s, pos, 0);
        uint32_t words = (uint32_t) (h < 0 ? -h : h);
        if (h < 0) printf("%u %d %d\n", pos, h, loft_st_i32(&s, pos + words - 1, 4));
        else printf("%u %d\n", pos, h);
        pos += words;
    }
    return 0;
}
"#,
    );
    let dir = scratch("store");
    let (src, ops, exe) = (dir.join("store.c"), dir.join("ops.txt"), dir.join("store"));
    fa::write(&src, &c).expect("write C");
    fa::write(&ops, &script).expect("write ops");
    let cc = loft::platform::process::harness_command("cc")
        .args(["-std=c99", "-O2", "-pedantic", "-Werror"])
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "cc rejected loft_store.h:\n{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .arg(&ops)
        .output()
        .expect("run the allocator program");
    let got = String::from_utf8_lossy(&out.stdout);
    if let Some((i, (w, g))) = want
        .lines()
        .zip(got.lines())
        .enumerate()
        .find(|(_, (w, g))| w != g)
    {
        panic!("line {i}: Store answers {w:?}, the C {g:?}");
    }
    assert_eq!(
        got.lines().count(),
        want.lines().count(),
        "the C stopped early"
    );
}

/// Stores::insert_record's hash arm, for an element that owns nothing: one walk files `e` or
/// finds the entry under its key, which `e` replaces; a walk that cannot answer falls back to a
/// lookup, a removal and an add.
fn hash_insert(
    stores: &mut [loft::store::Store],
    h: &loft::keys::DbRef,
    e: &loft::keys::DbRef,
    keys: &[loft::keys::Key],
) {
    use loft::hash::{self, Probed};
    match hash::probe_for_insert(h, e, stores, keys) {
        Probed::Free { bucket, index, tag } => hash::add_at(h, e, bucket, index, tag, stores, keys),
        Probed::Present {
            entry,
            bucket,
            index,
        } => {
            let gone = hash::replace_at(h, bucket, index, stores);
            hash_free_entry(stores, h, &entry, gone);
        }
        Probed::Unknown => {
            let key = loft::keys::get_key(e, stores, keys);
            let existing = hash::find(h, stores, keys, &key);
            if existing.rec != 0 && (existing.rec, existing.pos) != (e.rec, e.pos) {
                let slot = hash::remove(h, &existing, stores, keys);
                hash_free_entry(stores, h, &existing, slot);
            }
            hash::add(h, e, stores, keys);
        }
    }
}

/// Stores::free_hash_entry.
fn hash_free_entry(
    stores: &mut [loft::store::Store],
    h: &loft::keys::DbRef,
    e: &loft::keys::DbRef,
    slot: u32,
) {
    if slot == 0 {
        loft::hash::free_entry(h, e, stores);
    } else {
        loft::hash::free_slot(h, slot, stores);
    }
}

/// The hash table and its arena in C (`src/c/loft_hash.h`) are `hash.rs` and `arena.rs`: one
/// seeded sequence of inserts (a key already filed is replaced, as `Stores::insert_record`
/// replaces it), removals, lookups and reservations runs through both on three tables — an
/// integer key, a text key, and one that borrows its entries (stride 0, a linked type's) — and
/// every lookup, the walks in key order and in slot order, and every byte of the store agree —
/// and on a fourth whose first entry is a record it did not stage, which demotes it from an
/// inline table to an arena one.  Falsified: the load factor, the inline growth, the back-shift,
/// the insertion count, the arena's free list, a SipHash rotation, the text terminator and the
/// replace-in-place insert each fail it.
#[test]
fn the_c_hash_table_files_every_entry_where_hash_does() {
    use loft::hash;
    use loft::keys::{Content, DbRef, Key, Str};
    use loft::store::Store;
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let mut seed: u64 = 0x5eed_4a54_0000_0002;
    let mut next = |n: u64| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % n
    };
    let int_key = [Key {
        type_nr: 1,
        position: 0,
        start: 0,
    }];
    let text_key = [Key {
        type_nr: 6,
        position: 0,
        start: 0,
    }];
    let keys: [&[Key]; 4] = [&int_key, &text_key, &int_key, &int_key];
    let strides = [16u32, 16, 0, 16];
    let mut stores = vec![Store::new_in_use_defined(100)];
    let root = stores[0].claim(3);
    let h: Vec<DbRef> = (0..4)
        .map(|t| DbRef {
            store_nr: 0,
            rec: root,
            pos: 8 + 4 * t,
        })
        .collect();
    // A table's seed is drawn when it is made; both sides then take the same one.
    for t in 0..4 {
        let claim = hash::ensure_table(&h[t], strides[t], &mut stores);
        stores[0].set_u32_raw(claim, hash::SEED_FLD, 0x89ab_cdef);
        stores[0].set_u32_raw(claim, hash::SEED_FLD + 4, 0x0123_4567);
    }
    // A record the inline table did not stage: filing it demotes the table to an arena.
    let own = DbRef {
        store_nr: 0,
        rec: stores[0].claim(3),
        pos: 8,
    };
    stores[0].set_int(own.rec, 8, 1_000_000);
    stores[0].set_int(own.rec, 16, 7);
    hash_insert(&mut stores, &h[3], &own, keys[3]);
    let (mut script, mut want) = (String::new(), String::new());
    for _ in 0..6000 {
        let t = next(4) as usize;
        let pick = next(100);
        let k = next(if t == 1 { 200 } else { 300 });
        let name = format!("k{k}");
        if pick < 55 {
            let v = next(1_000_000) as i64;
            let e = if t == 2 {
                let rec = stores[0].claim(3);
                stores[0].set_u32_raw(rec, 4, root);
                DbRef {
                    store_nr: 0,
                    rec,
                    pos: 8,
                }
            } else {
                hash::alloc_entry(&h[t], strides[t], root, &mut stores)
            };
            if t == 1 {
                let text = stores[0].set_str(&name);
                stores[0].set_u32_raw(e.rec, e.pos, text);
            } else {
                stores[0].set_int(e.rec, e.pos, k as i64);
            }
            stores[0].set_int(e.rec, e.pos + 8, v);
            hash_insert(&mut stores, &h[t], &e, keys[t]);
            let _ = writeln!(script, "I {t} {k} {v}");
        } else {
            let key = if t == 1 {
                Content::Str(Str::new(&name))
            } else {
                Content::Long(k as i64)
            };
            let found = hash::find(&h[t], &stores, keys[t], &[key]);
            if pick < 75 {
                if found.rec != 0 {
                    let slot = hash::remove(&h[t], &found, &mut stores, keys[t]);
                    if strides[t] != 0 {
                        hash_free_entry(&mut stores, &h[t], &found, slot);
                    } else {
                        stores[0].delete(found.rec);
                    }
                }
                let _ = writeln!(script, "R {t} {k}");
            } else if pick < 97 {
                let _ = writeln!(script, "F {t} {k}");
                let _ = writeln!(want, "{} {}", found.rec, found.pos);
            } else {
                let n = next(400) as i64;
                hash::reserve(&h[t], n, strides[t], &mut stores, keys[t]);
                let _ = writeln!(script, "S {t} {n}");
            }
        }
    }
    for t in 0..4 {
        let _ = writeln!(want, "len {}", hash::count(&h[t], &stores));
        for e in hash::records_sorted(&h[t], &stores, keys[t]) {
            let _ = writeln!(want, "k {} {}", e.rec, e.pos);
        }
        for e in hash::records_by_slot(&h[t], &stores) {
            let _ = writeln!(want, "s {} {}", e.rec, e.pos);
        }
    }
    let store = &stores[0];
    let _ = writeln!(want, "size {}", store.capacity_words());
    let mut fnv: u64 = 0xcbf2_9ce4_8422_2325;
    for w in 0..store.capacity_words() {
        for b in store.read::<u64>(w, 0).to_le_bytes() {
            fnv = (fnv ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    let _ = writeln!(want, "bytes {fnv:016x}");
    let mut c = String::new();
    for part in [
        include_str!("../src/c/loft_cc.h"),
        include_str!("../src/c/loft_store.h"),
        include_str!("../src/c/loft_rt.h"),
        include_str!("../src/c/loft_keys.h"),
        include_str!("../src/c/loft_hash.h"),
        include_str!("../src/c/loft_btree.h"),
        include_str!("../src/c/loft_radix.h"),
        include_str!("../src/c/loft_radix_walk.h"),
        include_str!("../src/c/loft_radix_db.h"),
    ] {
        c.push_str(part);
    }
    c.push_str(
        r#"
static void loft_init(void) {}
static const loft_tdesc T[1] = { { LOFT_T_STRUCT, 16, 0, 0, 0, 0, 0, 0, 0, 0 } };
static const loft_key K_INT[1] = { { 1, 0, 0 } };
static const loft_key K_TEXT[1] = { { 6, 0, 0 } };

int main(int argc, char **argv) {
    static loft_store_t st;
    st.size = 100;
    st.buf = (uint8_t *) calloc(st.size, 8);
    st.defined = 1;
    st.live = 1;
    loft_st_init(&st);
    loft_stores = &st;
    loft_nstores = 1;
    const loft_key *keys[4] = { K_INT, K_TEXT, K_INT, K_INT };
    uint32_t strides[4] = { 16, 16, 0, 16 };
    uint32_t root = loft_st_claim(&st, 3);
    loft_ref h[4];
    for (int t = 0; t < 4; t++) {
        h[t].store_nr = 0;
        h[t].rec = root;
        h[t].pos = 8 + 4 * (uint32_t) t;
        loft_h_write_seed(&st, loft_h_ensure_table(h[t], strides[t]), UINT64_C(0x0123456789abcdef));
    }
    loft_ref own = { 0, 0, 8 };
    own.rec = loft_st_claim(&st, 3);
    int64_t own_key = 1000000, own_value = 7;
    memcpy(loft_at(&st, own.rec, 8), &own_key, 8);
    memcpy(loft_at(&st, own.rec, 16), &own_value, 8);
    (void) loft_hash_insert(T, 0, h[3], own, K_INT, 1, 0);
    FILE *f = fopen(argv[argc - 1], "r");
    char op;
    unsigned t, k;
    long long v;
    while (fscanf(f, " %c %u", &op, &t) == 2) {
        char name[32];
        int len;
        if (op == 'S') {
            if (fscanf(f, "%lld", &v) != 1) return 2;
            loft_h_reserve(h[t], v, strides[t], keys[t], 1);
            continue;
        }
        if (fscanf(f, "%u", &k) != 1) return 2;
        len = sprintf(name, "k%u", k);
        if (op == 'I') {
            if (fscanf(f, "%lld", &v) != 1) return 2;
            loft_ref e = loft_hash_entry(h[t], 16, t == 2);
            if (t == 1) {
                uint32_t text = loft_st_claim(&st, (uint32_t) (len + 15) / 8);
                loft_set_u32(&st, text, 4, (uint32_t) len);
                memcpy(loft_at(&st, text, 8), name, (size_t) len);
                loft_set_u32(&st, e.rec, e.pos, text);
            } else {
                int64_t kk = k;
                memcpy(loft_at(&st, e.rec, e.pos), &kk, 8);
            }
            int64_t vv = v;
            memcpy(loft_at(&st, e.rec, e.pos + 8), &vv, 8);
            (void) loft_hash_insert(T, 0, h[t], e, keys[t], 1, 0);
            continue;
        }
        loft_kv kv = t == 1 ? loft_kv_text(loft_lit(name, (size_t) len)) : loft_kv_int(k);
        loft_ref found = loft_h_find(h[t], keys[t], &kv, 1);
        if (op == 'R') {
            if (found.rec != 0) loft_hash_remove(T, 0, h[t], found, keys[t], 1, 0);
        } else {
            printf("%u %u\n", found.rec, found.pos);
        }
    }
    for (int t = 0; t < 4; t++) {
        uint32_t n;
        printf("len %lld\n", (long long) loft_hash_len(h[t]));
        for (int by_slot = 0; by_slot < 2; by_slot++) {
            loft_ref *e = loft_hash_refs(h[t], keys[t], 1, by_slot, &n);
            for (uint32_t i = 0; i < n; i++) printf("%c %u %u\n", by_slot ? 's' : 'k', e[i].rec, e[i].pos);
            free(e);
        }
    }
    printf("size %u\n", st.size);
    uint64_t fnv = UINT64_C(0xcbf29ce484222325);
    for (size_t i = 0; i < (size_t) st.size * 8; i++) fnv = (fnv ^ st.buf[i]) * UINT64_C(0x100000001b3);
    printf("bytes %016llx\n", (unsigned long long) fnv);
    return 0;
}
"#,
    );
    let dir = scratch("hash");
    let (src, ops, exe) = (dir.join("hash.c"), dir.join("ops.txt"), dir.join("hash"));
    fa::write(&src, &c).expect("write C");
    fa::write(&ops, &script).expect("write ops");
    let cc = loft::platform::process::harness_command("cc")
        .args([
            "-std=c99",
            "-O2",
            "-pedantic",
            "-Werror",
            "-Wno-unused-function",
        ])
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "cc rejected loft_hash.h:\n{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .arg(&ops)
        .output()
        .expect("run the hash program");
    let got = String::from_utf8_lossy(&out.stdout);
    if let Some((i, (w, g))) = want
        .lines()
        .zip(got.lines())
        .enumerate()
        .find(|(_, (w, g))| w != g)
    {
        panic!("line {i}: hash.rs answers {w:?}, the C {g:?}");
    }
    assert_eq!(
        got.lines().count(),
        want.lines().count(),
        "the C stopped early: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The index in C (`src/c/loft_btree.h`) is `btree.rs`: one seeded sequence of inserts (a key
/// already filed displaces its record, as `Stores::insert_record` does), removals, exact and
/// partial lookups and bounded walks runs through both on three indexes — an integer key, a text
/// key whose values share prefixes (the nodes' prefixes, kept on ties), and a two-part key whose
/// first part descends (the descent that compares records) — and every answer, every walk and
/// every byte of the store agree.  Falsified: the rebalance threshold, both split points, the
/// prefix taken on ties, a descending part, a merge that keeps its node, the lookup below a key
/// and the arena's growth each fail it.  (A place that refits without forcing is no mutant: a key
/// outside a node's prefix lands at its edge, so the shared prefix shrinks and the refit runs.)
#[test]
fn the_c_index_files_every_record_where_btree_does() {
    use loft::btree;
    use loft::keys::{Content, DbRef, Key, Str};
    use loft::store::Store;
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let mut seed: u64 = 0x5eed_b7ee_0000_0003;
    let mut next = |n: u64| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % n
    };
    let key = |type_nr: i8, position: u16| Key {
        type_nr,
        position,
        start: 0,
    };
    let int_key = [key(1, 0)];
    let text_key = [key(6, 0)];
    let pair_key = [key(-1, 8), key(1, 0)];
    let keys: [&[Key]; 3] = [&int_key, &text_key, &pair_key];
    let mut stores = vec![Store::new_in_use_defined(100)];
    let root = stores[0].claim(3);
    let ix: Vec<DbRef> = (0..3)
        .map(|t| DbRef {
            store_nr: 0,
            rec: root,
            pos: 8 + 4 * t,
        })
        .collect();
    // Three kinds of name: short ones, ones sharing a prefix longer than a node keeps, and ones
    // whose first eight bytes a third of them share (a node's ties between a quarter and half).
    let name = |k: u64| {
        if k.is_multiple_of(7) {
            format!("k{k}")
        } else if k.is_multiple_of(3) {
            format!("a-long-shared-prefix-of-names-{k:05}")
        } else {
            format!("p{:07}x{k}", k / 2)
        }
    };
    // The key values of record key `k` in index `t`, as a lookup takes them.
    let values = |t: usize, k: u64, parts: usize| -> Vec<Content> {
        match t {
            0 => vec![Content::Long(k as i64)],
            1 => vec![Content::Str(Str::new(Box::leak(name(k).into_boxed_str())))],
            _ => [Content::Long((k % 10) as i64), Content::Long(k as i64)][..parts].to_vec(),
        }
    };
    let (mut script, mut want) = (String::new(), String::new());
    for _ in 0..6000 {
        let t = next(3) as usize;
        let pick = next(100);
        let k = next(400);
        if pick < 50 {
            let rec = stores[0].claim(3);
            stores[0].set_u32_raw(rec, 4, root);
            if t == 1 {
                let text = stores[0].set_str(&name(k));
                stores[0].set_u32_raw(rec, 8, text);
            } else {
                stores[0].set_int(rec, 8, k as i64);
            }
            stores[0].set_int(rec, 16, (k % 10) as i64);
            // Stores::insert_record's index arm: a refused add names the record it keeps.
            let dup = btree::add(&ix[t], rec, &mut stores, keys[t]);
            if dup != 0 {
                btree::remove(&ix[t], dup, &mut stores, keys[t]);
                stores[0].delete(dup);
                btree::add(&ix[t], rec, &mut stores, keys[t]);
            }
            let _ = writeln!(script, "I {t} {k}");
        } else if pick < 70 {
            let found = btree::find_exact(&ix[t], &stores, keys[t], &values(t, k, 2));
            if found != 0 {
                btree::remove(&ix[t], found, &mut stores, keys[t]);
                stores[0].delete(found);
            }
            let _ = writeln!(script, "R {t} {k}");
        } else if pick < 85 {
            let found = btree::find_exact(&ix[t], &stores, keys[t], &values(t, k, 2));
            let below = btree::find_record(&ix[t], true, &stores, keys[t], &values(t, k, 1));
            let _ = writeln!(script, "F {t} {k}");
            let _ = writeln!(want, "{found} {below}");
        } else {
            let till = k + next(60);
            let ex = next(2) == 1;
            let walk = btree::range(
                &ix[t],
                &stores,
                keys[t],
                &values(t, k, 1),
                &values(t, till, 1),
                ex,
            );
            let _ = writeln!(script, "W {t} {k} {till} {}", u8::from(ex));
            let mut line = String::from("w");
            if let Some((low, high)) = walk {
                let mut p = low;
                loop {
                    let _ = write!(line, " {}", btree::record(&ix[t], &stores, p));
                    if p == high || p == 0 {
                        break;
                    }
                    p = btree::next(&ix[t], &stores, p);
                }
            }
            let _ = writeln!(want, "{line}");
        }
    }
    for one in &ix {
        let all = btree::records(one, &stores);
        let _ = writeln!(
            want,
            "len {} {}",
            btree::count(one, &stores),
            all.iter().map(u32::to_string).collect::<Vec<_>>().join(" ")
        );
    }
    let store = &stores[0];
    let _ = writeln!(want, "size {}", store.capacity_words());
    let mut fnv: u64 = 0xcbf2_9ce4_8422_2325;
    for w in 0..store.capacity_words() {
        for b in store.read::<u64>(w, 0).to_le_bytes() {
            fnv = (fnv ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    let _ = writeln!(want, "bytes {fnv:016x}");
    let mut c = String::new();
    for part in [
        include_str!("../src/c/loft_cc.h"),
        include_str!("../src/c/loft_store.h"),
        include_str!("../src/c/loft_rt.h"),
        include_str!("../src/c/loft_keys.h"),
        include_str!("../src/c/loft_hash.h"),
        include_str!("../src/c/loft_btree.h"),
        include_str!("../src/c/loft_radix.h"),
        include_str!("../src/c/loft_radix_walk.h"),
        include_str!("../src/c/loft_radix_db.h"),
    ] {
        c.push_str(part);
    }
    c.push_str(
        r#"
static void loft_init(void) {}
static const loft_tdesc T[1] = { { LOFT_T_STRUCT, 24, 0, 0, 0, 0, 0, 0, 0, 0 } };
static const loft_key K_INT[1] = { { 1, 0, 0 } };
static const loft_key K_TEXT[1] = { { 6, 0, 0 } };
static const loft_key K_PAIR[2] = { { -1, 8, 0 }, { 1, 0, 0 } };
static loft_store_t st;

static int name_of(char *out, unsigned k) {
    if (k % 7 == 0) return sprintf(out, "k%u", k);
    if (k % 3 == 0) return sprintf(out, "a-long-shared-prefix-of-names-%05u", k);
    return sprintf(out, "p%07ux%u", k / 2, k);
}

static int values(int t, unsigned k, int parts, char *buf, loft_kv *kv) {
    if (t == 0) { kv[0] = loft_kv_int(k); return 1; }
    if (t == 1) { int len = name_of(buf, k); kv[0] = loft_kv_text(loft_lit(buf, (size_t) len)); return 1; }
    kv[0] = loft_kv_int(k % 10);
    kv[1] = loft_kv_int(k);
    return parts;
}

int main(int argc, char **argv) {
    st.size = 100;
    st.buf = (uint8_t *) calloc(st.size, 8);
    st.defined = 1;
    st.live = 1;
    loft_st_init(&st);
    loft_stores = &st;
    loft_nstores = 1;
    const loft_key *keys[3] = { K_INT, K_TEXT, K_PAIR };
    int nks[3] = { 1, 1, 2 };
    uint32_t root = loft_st_claim(&st, 3);
    loft_ref ix[3];
    for (int t = 0; t < 3; t++) {
        ix[t].store_nr = 0;
        ix[t].rec = root;
        ix[t].pos = 8 + 4 * (uint32_t) t;
    }
    FILE *f = fopen(argv[argc - 1], "r");
    char op;
    unsigned t, k, till, ex;
    while (fscanf(f, " %c %u %u", &op, &t, &k) == 3) {
        char buf[64], buf2[64];
        loft_kv kv[2], kv2[2];
        if (op == 'I') {
            loft_ref e = { 0, loft_st_claim(&st, 3), 8 };
            loft_set_u32(&st, e.rec, 4, root);
            if (t == 1) {
                int len = name_of(buf, k);
                uint32_t text = loft_st_claim(&st, (uint32_t) (len + 15) / 8);
                loft_set_u32(&st, text, 4, (uint32_t) len);
                memcpy(loft_at(&st, text, 8), buf, (size_t) len);
                loft_set_u32(&st, e.rec, 8, text);
            } else {
                int64_t kk = k;
                memcpy(loft_at(&st, e.rec, 8), &kk, 8);
            }
            int64_t vv = k % 10;
            memcpy(loft_at(&st, e.rec, 16), &vv, 8);
            (void) loft_ix_insert(T, 0, ix[t], e, keys[t], nks[t], 0);
        } else if (op == 'R') {
            int n = values((int) t, k, 2, buf, kv);
            loft_ref found = loft_ix_find(ix[t], keys[t], nks[t], kv, n);
            if (found.rec != 0) loft_ix_remove(T, 0, ix[t], found, keys[t], nks[t], 0);
        } else if (op == 'F') {
            int n = values((int) t, k, 2, buf, kv);
            uint32_t found = loft_bt_find_exact(ix[t], keys[t], nks[t], kv, n);
            n = values((int) t, k, 1, buf, kv);
            uint32_t below = loft_bt_record(ix[t], loft_bt_find(ix[t], 1, keys[t], nks[t], kv, n));
            printf("%u %u\n", found, below);
        } else {
            uint32_t low, high;
            if (fscanf(f, "%u %u", &till, &ex) != 2) return 2;
            int n = values((int) t, k, 1, buf, kv);
            int n2 = values((int) t, till, 1, buf2, kv2);
            printf("w");
            if (loft_bt_range(ix[t], keys[t], nks[t], kv, n, kv2, n2, (int) ex, &low, &high)) {
                for (uint32_t p = low;;) {
                    printf(" %u", loft_bt_record(ix[t], p));
                    if (p == high || p == 0) break;
                    p = loft_bt_next(ix[t], p);
                }
            }
            printf("\n");
        }
    }
    for (int t = 0; t < 3; t++) {
        uint32_t n;
        uint32_t *all = loft_bt_records(ix[t], &n);
        printf("len %u", loft_bt_len(ix[t]));
        for (uint32_t i = 0; i < n; i++) printf(" %u", all[i]);
        printf("\n");
        free(all);
    }
    printf("size %u\n", st.size);
    uint64_t fnv = UINT64_C(0xcbf29ce484222325);
    for (size_t i = 0; i < (size_t) st.size * 8; i++) fnv = (fnv ^ st.buf[i]) * UINT64_C(0x100000001b3);
    printf("bytes %016llx\n", (unsigned long long) fnv);
    return 0;
}
"#,
    );
    let dir = scratch("btree");
    let (src, ops, exe) = (dir.join("btree.c"), dir.join("ops.txt"), dir.join("btree"));
    fa::write(&src, &c).expect("write C");
    fa::write(&ops, &script).expect("write ops");
    let cc = loft::platform::process::harness_command("cc")
        .args([
            "-std=c99",
            "-O2",
            "-pedantic",
            "-Werror",
            "-Wno-unused-function",
        ])
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "cc rejected loft_btree.h:\n{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .arg(&ops)
        .output()
        .expect("run the index program");
    let got = String::from_utf8_lossy(&out.stdout);
    if let Some((i, (w, g))) = want
        .lines()
        .zip(got.lines())
        .enumerate()
        .find(|(_, (w, g))| w != g)
    {
        panic!("line {i}: btree.rs answers {w:?}, the C {g:?}");
    }
    assert_eq!(
        got.lines().count(),
        want.lines().count(),
        "the C stopped early: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The spatial and trie collections in C (`src/c/loft_radix.h`) are `radix_tree.rs` under
/// `radix_db.rs`' Morton oracle and `trie_db.rs`' text oracle: one seeded sequence of point
/// inserts and removals, word inserts (a repeated word displaces its record, as
/// `Stores::insert_record` does for a trie) and removals, lookups, outward walks, box walks and
/// prefix walks runs through both, and every answer, both trees' order and every byte of the
/// store agree.  Points repeat, so records share a cell and differ only in their id suffix.
/// Falsified: the terminator width, the axis interleave, the coordinate sign flip, the outward
/// walk's tie, the tree growth, the free-node list, BIGMIN's low update, the seek's step past,
/// the sibling's parent link and the trie's displacement each fail it.
#[test]
fn the_c_trees_place_every_record_where_radix_tree_does() {
    use loft::keys::{Content, DbRef, Key, Str};
    use loft::store::Store;
    use loft::{radix_db, trie_db};
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let mut seed: u64 = 0x5eed_7ad1_0000_0004;
    let mut next = |n: u64| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % n
    };
    let key = |type_nr: i8, position: u16| Key {
        type_nr,
        position,
        start: 0,
    };
    let axes = [key(1, 0), key(1, 8)];
    let word_key = [key(6, 0)];
    let mut stores = vec![Store::new_in_use_defined(100)];
    let root = stores[0].claim(3);
    let pts = DbRef {
        store_nr: 0,
        rec: root,
        pos: 8,
    };
    let words = DbRef {
        store_nr: 0,
        rec: root,
        pos: 12,
    };
    // Coordinates from -20..20 so the sign bit and shared cells both occur; words with shared
    // prefixes, so prefix walks and the terminator ordering ("ab" before "abc") both occur.
    let word = |k: u64| match k % 3 {
        0 => format!("w{}", k / 3),
        1 => format!("w{}x", k / 3),
        _ => format!("prefix{k}"),
    };
    let text = |w: String| Content::Str(Str::new(Box::leak(w.into_boxed_str())));
    let point = |x: i64, y: i64| [Content::Long(x), Content::Long(y)];
    let ids = |recs: Vec<u32>| recs.iter().map(|r| format!(" {r}")).collect::<String>();
    let (mut script, mut want) = (String::new(), String::new());
    for _ in 0..5000 {
        let pick = next(100);
        let (x, y) = (next(40) as i64 - 20, next(40) as i64 - 20);
        let k = next(150);
        if pick < 30 {
            let rec = stores[0].claim(3);
            stores[0].set_u32_raw(rec, 4, root);
            stores[0].set_int(rec, 8, x);
            stores[0].set_int(rec, 16, y);
            radix_db::add(
                &pts,
                &DbRef {
                    store_nr: 0,
                    rec,
                    pos: 8,
                },
                &mut stores,
                &axes,
            );
            let _ = writeln!(script, "P {x} {y}");
        } else if pick < 40 {
            let found = radix_db::find(&pts, &stores, &axes, &point(x, y));
            if found.rec != 0 {
                radix_db::remove(&pts, &found, &mut stores, &axes);
                stores[0].delete(found.rec);
            }
            let _ = writeln!(script, "Q {x} {y}");
        } else if pick < 65 {
            let rec = stores[0].claim(2);
            stores[0].set_u32_raw(rec, 4, root);
            let t = stores[0].set_str(&word(k));
            stores[0].set_u32_raw(rec, 8, t);
            let e = DbRef {
                store_nr: 0,
                rec,
                pos: 8,
            };
            // Stores::dedup_keyed for a trie: the record under the word goes first.
            let existing = trie_db::find(&words, &stores, &word_key, &[text(word(k))]);
            if existing.rec != 0 && existing.rec != rec {
                trie_db::remove(&words, &existing, &mut stores, &word_key);
                stores[0].delete(existing.rec);
            }
            trie_db::add(&words, &e, &mut stores, &word_key);
            let _ = writeln!(script, "T {k}");
        } else if pick < 72 {
            let found = trie_db::find(&words, &stores, &word_key, &[text(word(k))]);
            if found.rec != 0 {
                trie_db::remove(&words, &found, &mut stores, &word_key);
                stores[0].delete(found.rec);
            }
            let _ = writeln!(script, "U {k}");
        } else if pick < 82 {
            let p = radix_db::find(&pts, &stores, &axes, &point(x, y)).rec;
            let w = trie_db::find(&words, &stores, &word_key, &[text(word(k))]).rec;
            let _ = writeln!(script, "F {x} {y} {k}");
            let _ = writeln!(want, "{p} {w}");
        } else if pick < 88 {
            let lim = next(12);
            let near = radix_db::near_range(&pts, &stores, &axes, &[x, y], Some(lim as usize));
            let _ = writeln!(script, "N {x} {y} {lim}");
            let _ = writeln!(want, "n{}", ids(near));
        } else if pick < 94 {
            let (tx, ty) = (x + next(15) as i64 - 3, y + next(15) as i64 - 3);
            let found = radix_db::box_range(&pts, &stores, &axes, &[x, y], &[tx, ty], None);
            let _ = writeln!(script, "B {x} {y} {tx} {ty}");
            let _ = writeln!(want, "b{}", ids(found));
        } else {
            let pre = word(k);
            let pre = &pre[..pre.len().min(1 + next(3) as usize)];
            let found = trie_db::prefix(&words, &stores, &word_key, pre.as_bytes(), None);
            let _ = writeln!(script, "X {pre}");
            let _ = writeln!(want, "x{}", ids(found));
        }
    }
    let _ = writeln!(want, "p{}", ids(radix_db::records(&pts, &stores)));
    let _ = writeln!(want, "t{}", ids(trie_db::records(&words, &stores)));
    let store = &stores[0];
    let _ = writeln!(want, "size {}", store.capacity_words());
    let mut fnv: u64 = 0xcbf2_9ce4_8422_2325;
    for w in 0..store.capacity_words() {
        for b in store.read::<u64>(w, 0).to_le_bytes() {
            fnv = (fnv ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    let _ = writeln!(want, "bytes {fnv:016x}");
    let mut c = String::new();
    for part in [
        include_str!("../src/c/loft_cc.h"),
        include_str!("../src/c/loft_store.h"),
        include_str!("../src/c/loft_rt.h"),
        include_str!("../src/c/loft_keys.h"),
        include_str!("../src/c/loft_hash.h"),
        include_str!("../src/c/loft_btree.h"),
        include_str!("../src/c/loft_radix.h"),
        include_str!("../src/c/loft_radix_walk.h"),
        include_str!("../src/c/loft_radix_db.h"),
    ] {
        c.push_str(part);
    }
    c.push_str(
        r#"
static void loft_init(void) {}
static const loft_tdesc T[1] = { { LOFT_T_STRUCT, 16, 0, 0, 0, 0, 0, 0, 0, 0 } };
static const loft_key K_AXES[2] = { { 1, 0, 0 }, { 1, 8, 0 } };
static const loft_key K_WORD[1] = { { 6, 0, 0 } };
static loft_store_t st;

static int word_of(char *out, unsigned k) {
    if (k % 3 == 0) return sprintf(out, "w%u", k / 3);
    if (k % 3 == 1) return sprintf(out, "w%ux", k / 3);
    return sprintf(out, "prefix%u", k);
}

static void ids(const char *tag, uint32_t *r, uint32_t n) {
    printf("%s", tag);
    for (uint32_t i = 0; i < n; i++) printf(" %u", r[i]);
    printf("\n");
    free(r);
}

int main(int argc, char **argv) {
    st.size = 100;
    st.buf = (uint8_t *) calloc(st.size, 8);
    st.defined = 1;
    st.live = 1;
    loft_st_init(&st);
    loft_stores = &st;
    loft_nstores = 1;
    uint32_t root = loft_st_claim(&st, 3);
    loft_ref pts = { 0, root, 8 }, words = { 0, root, 12 };
    FILE *f = fopen(argv[argc - 1], "r");
    char op, buf[64];
    long long x, y, tx, ty;
    unsigned k, lim;
    uint32_t n;
    while (fscanf(f, " %c", &op) == 1) {
        if (op == 'P' || op == 'Q') {
            if (fscanf(f, "%lld %lld", &x, &y) != 2) return 2;
            if (op == 'P') {
                loft_ref e = { 0, loft_st_claim(&st, 3), 8 };
                int64_t xx = x, yy = y;
                loft_set_u32(&st, e.rec, 4, root);
                memcpy(loft_at(&st, e.rec, 8), &xx, 8);
                memcpy(loft_at(&st, e.rec, 16), &yy, 8);
                (void) loft_rx_insert_record(T, 0, pts, e, K_AXES, 2, 0, 0);
            } else {
                loft_kv kv[2] = { loft_kv_int(x), loft_kv_int(y) };
                loft_ref found = loft_rx_find(pts, K_AXES, 2, 0, kv, 2);
                if (found.rec != 0) loft_rx_remove_owned(T, 0, pts, found, K_AXES, 2, 0, 0);
            }
        } else if (op == 'T' || op == 'U') {
            if (fscanf(f, "%u", &k) != 1) return 2;
            int len = word_of(buf, k);
            if (op == 'T') {
                loft_ref e = { 0, loft_st_claim(&st, 2), 8 };
                loft_set_u32(&st, e.rec, 4, root);
                uint32_t t = loft_st_claim(&st, (uint32_t) (len + 15) / 8);
                loft_set_u32(&st, t, 4, (uint32_t) len);
                memcpy(loft_at(&st, t, 8), buf, (size_t) len);
                loft_set_u32(&st, e.rec, 8, t);
                (void) loft_rx_insert_record(T, 0, words, e, K_WORD, 1, 1, 0);
            } else {
                loft_kv kv = loft_kv_text(loft_lit(buf, (size_t) len));
                loft_ref found = loft_rx_find(words, K_WORD, 1, 1, &kv, 1);
                if (found.rec != 0) loft_rx_remove_owned(T, 0, words, found, K_WORD, 1, 1, 0);
            }
        } else if (op == 'F') {
            if (fscanf(f, "%lld %lld %u", &x, &y, &k) != 3) return 2;
            loft_kv kv[2] = { loft_kv_int(x), loft_kv_int(y) };
            int len = word_of(buf, k);
            loft_kv kw = loft_kv_text(loft_lit(buf, (size_t) len));
            printf("%u %u\n", loft_rx_find(pts, K_AXES, 2, 0, kv, 2).rec, loft_rx_find(words, K_WORD, 1, 1, &kw, 1).rec);
        } else if (op == 'N') {
            if (fscanf(f, "%lld %lld %u", &x, &y, &lim) != 3) return 2;
            int64_t from[2] = { x, y };
            ids("n", loft_rx_near(pts, K_AXES, 2, from, lim, &n), n);
        } else if (op == 'B') {
            if (fscanf(f, "%lld %lld %lld %lld", &x, &y, &tx, &ty) != 4) return 2;
            int64_t from[2] = { x, y }, till[2] = { tx, ty };
            ids("b", loft_rx_box(pts, K_AXES, 2, from, till, SIZE_MAX, &n), n);
        } else if (op == 'X') {
            if (fscanf(f, "%63s", buf) != 1) return 2;
            ids("x", loft_rx_prefix(words, K_WORD, loft_lit(buf, strlen(buf)), SIZE_MAX, &n), n);
        }
    }
    ids("p", loft_rx_records(pts, &n), n);
    ids("t", loft_rx_records(words, &n), n);
    printf("size %u\n", st.size);
    uint64_t fnv = UINT64_C(0xcbf29ce484222325);
    for (size_t i = 0; i < (size_t) st.size * 8; i++) fnv = (fnv ^ st.buf[i]) * UINT64_C(0x100000001b3);
    printf("bytes %016llx\n", (unsigned long long) fnv);
    return 0;
}
"#,
    );
    let dir = scratch("radix");
    let (src, ops, exe) = (dir.join("radix.c"), dir.join("ops.txt"), dir.join("radix"));
    fa::write(&src, &c).expect("write C");
    fa::write(&ops, &script).expect("write ops");
    let cc = loft::platform::process::harness_command("cc")
        .args([
            "-std=c99",
            "-O2",
            "-pedantic",
            "-Werror",
            "-Wno-unused-function",
        ])
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "cc rejected loft_radix.h:\n{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .arg(&ops)
        .output()
        .expect("run the tree program");
    let got = String::from_utf8_lossy(&out.stdout);
    if let Some((i, (w, g))) = want
        .lines()
        .zip(got.lines())
        .enumerate()
        .find(|(_, (w, g))| w != g)
    {
        panic!("line {i}: radix_tree.rs answers {w:?}, the C {g:?}");
    }
    assert_eq!(
        got.lines().count(),
        want.lines().count(),
        "the C stopped early: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The program a store image is written and read with: every collection kind in one root, its
/// reader rendering the loaded value whole (a walk by type, never the raw buffer) and querying
/// each kind — a lookup, an outward spatial walk, a trie prefix walk.
const IMAGE_PROGRAM: &str = r#"struct Rec { id: integer, val: integer, name: text }
struct Pt { x: integer, y: integer, tag: text }
struct Ix { tag: text, z: integer }
struct Wd { word: text, n: integer }
struct Tw { word: text, n: integer }
struct Sv { n: integer, s: text }
struct W { recs: hash<Rec[id]>, ix: index<Ix[tag]>, pts: spatial<Pt[x, y]>, all: vector<Wd>, t: trie<Tw[word]>, s: sorted<Sv[n]> }

fn main() {
  path = env_variable("LOFT_PERSIST_TEST_PATH");
  mode = env_variable("LOFT_PERSIST_TEST_MODE");
  w = W {};
  if mode == "write" {
    for i in 0..60 {
      w.recs += Rec { id: i * 7 % 53, val: i, name: "r{i % 9}" };
      w.ix += Ix { tag: "k{i}", z: -i };
      w.pts += Pt { x: i % 5 - 2, y: i / 5, tag: "p{i}" };
      w.all += [Wd { word: "w{i}", n: i }];
      w.t += Tw { word: "word{i % 17}x{i}", n: i };
      w.s += Sv { n: 100 - i, s: "s{i}" };
    }
    if !store_persist_copy(w, path) { println("FAIL write"); return }
    small: hash<Rec[id]> = [Rec { id: 1, val: 2, name: "one" }, Rec { id: 3, val: 4, name: "three" }];
    if !store_persist_copy(small, "{path}.small") { println("FAIL small"); return }
    println("written");
  } else {
    if !store_load(w, path) { println("FAIL load"); return }
    small: hash<Rec[id]> = [];
    if !store_load(small, "{path}.small") { println("FAIL small"); return }
    println("{w}");
    sm = "";
    for r in small { sm += "{r.id}:{r.val}:{r.name} "; }
    println("small {sm}");
    println("lookups {w.recs[13].name} {w.ix["k7"].z} {w.t["word3x20"].n} {len(w.pts)} {len(w.s)}");
    out = "";
    for p in w.pts[(0, 3)..:4] { out += "{p.tag} "; }
    println("near {out}");
    pre = "";
    for t in w.t["word1"..] { pre += "{t.n} "; }
    println("prefix {pre}");
  }
}
"#;

/// A program written as C, beside the loft binary: run as each backend it is crossed on — the
/// interpreter, the C, and `--native` where `rustc` is on PATH — with the image test's mode and
/// path in its environment and a fixed hash seed, so a hash image is byte-reproducible.
struct Crossed {
    loft: &'static str,
    src: std::path::PathBuf,
    exe: std::path::PathBuf,
    backends: Vec<&'static str>,
}

impl Crossed {
    /// `None` when there is no `cc` on PATH.
    fn new(tag: &str, program: &str) -> Option<Crossed> {
        if !have_cc() {
            eprintln!("skipped: no cc on PATH");
            return None;
        }
        let loft = env!("CARGO_BIN_EXE_loft");
        let dir = scratch(tag);
        let src = dir.join("rw.loft");
        fa::write(&src, program).expect("write program");
        let (c_src, exe) = (dir.join("rw.c"), dir.join("rw"));
        let emit = loft::platform::process::harness_command(loft)
            .arg("--emit-c")
            .arg(&c_src)
            .arg(&src)
            .output()
            .expect("run loft --emit-c");
        assert!(
            emit.status.success(),
            "emit: {}",
            String::from_utf8_lossy(&emit.stderr)
        );
        let cc = loft::platform::process::harness_command("cc")
            .args(["-std=c99", "-O1", "-w"])
            .arg(&c_src)
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .output()
            .expect("run cc");
        assert!(
            cc.status.success(),
            "cc: {}",
            String::from_utf8_lossy(&cc.stderr)
        );
        let native = loft::platform::process::harness_command("rustc")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        let mut backends = vec!["interpret", "c"];
        if native {
            backends.push("native");
        }
        Some(Crossed {
            loft,
            src,
            exe,
            backends,
        })
    }

    fn dir(&self) -> &std::path::Path {
        self.src.parent().expect("the program's directory")
    }

    fn run(&self, backend: &str, mode: &str, path: &std::path::Path) -> String {
        let mut cmd = if backend == "c" {
            loft::platform::process::harness_command(&self.exe)
        } else {
            let mut c = loft::platform::process::harness_command(self.loft);
            c.arg(format!("--{backend}")).arg(&self.src);
            c
        };
        let out = cmd
            .env("LOFT_PERSIST_TEST_MODE", mode)
            .env("LOFT_PERSIST_TEST_PATH", path)
            .env("LOFT_HASH_SEED", "0x0123456789abcdef")
            .env("LOFT_TIMEOUT", "300")
            .output()
            .expect("run a backend");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

/// The owner's acceptance for the image (@PLN200 B5 step (e)): a store file written by any
/// backend is read by every backend — the interpreter, the C, and `--native` where `rustc` is
/// on PATH — and each reader renders the same data.  The C's image is also the interpreter's,
/// byte for byte (a fixed hash seed; the sidecar's slot line names each writer's own scratch).
#[test]
fn a_store_image_crosses_every_backend() {
    let Some(x) = Crossed::new("image", IMAGE_PROGRAM) else {
        return;
    };
    let (dir, backends) = (x.dir(), &x.backends);
    let run = |backend: &str, mode: &str, path: &std::path::Path| x.run(backend, mode, path);
    for w in backends {
        assert_eq!(
            run(w, "write", &dir.join(format!("{w}.store"))),
            "written\n",
            "{w} writes"
        );
    }
    let want = run("interpret", "load", &dir.join("interpret.store"));
    assert!(
        want.contains("lookups r8 -7 20 60 60"),
        "the reader answers: {want}"
    );
    assert!(
        want.contains("small 1:2:one 3:4:three"),
        "the small store reads back: {want}"
    );
    for w in backends {
        for r in backends {
            let got = run(r, "load", &dir.join(format!("{w}.store")));
            assert_eq!(got, want, "{r} reading {w}'s image");
        }
    }
    // Both images — one past the image floor, one padded up to it — and the sidecars, whose
    // one line that may differ names the slot each writer's scratch store took.
    let read = |name: String| fa::read(dir.join(name)).expect("read a written file");
    for suffix in ["store", "store.small"] {
        assert_eq!(
            read(format!("c.{suffix}")),
            read(format!("interpret.{suffix}")),
            "the C's {suffix} image is the interpreter's"
        );
        let sidecar = |w: &str| -> String {
            String::from_utf8_lossy(&read(format!("{w}.{suffix}.dschema")))
                .lines()
                .map(|l| if l.starts_with("slot=") { "slot=" } else { l })
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(
            sidecar("c"),
            sidecar("interpret"),
            "the {suffix} sidecars agree"
        );
        assert!(
            sidecar("c").contains("\nslot="),
            "the C's {suffix} sidecar names its slot"
        );
    }
}

/// The program the paged loaders are crossed with: a hash entry holding a text, a vector of
/// integers, a nested struct and a vector of structs; a text-keyed hash; a trie; a spatial index;
/// a sorted; an entry type the working-set copy refuses (`vector<text>`); and a three-entry hash
/// (an inline table, where the forty-entry one has an arena).  Each loader asked once or twice.
const PAGED_PROGRAM: &str = r#"struct Item { tag: text, n: integer }
struct Rec { id: integer, name: text, tags: vector<integer>, sub: Item, items: vector<Item> }
struct Word { w: text, n: integer }
struct Pt { x: integer, y: integer, label: text }
struct Sv { k: integer, s: text }
struct Bad { id: integer, names: vector<text> }
struct Small { id: integer, v: integer }

fn main() {
  path = env_variable("LOFT_PERSIST_TEST_PATH");
  mode = env_variable("LOFT_PERSIST_TEST_MODE");
  h: hash<Rec[id]> = [];
  th: hash<Word[w]> = [];
  tr: trie<Word[w]> = [];
  sp: spatial<Pt[x, y]> = [];
  so: sorted<Sv[k]> = [];
  bad: hash<Bad[id]> = [];
  sm: hash<Small[id]> = [];
  if mode == "write" {
    for i in 0..40 {
      h += Rec { id: i * 3, name: "rec{i}", tags: [i, i + 1], sub: Item { tag: "s{i}", n: -i }, items: [Item { tag: "a{i}", n: i }, Item { tag: "b{i}", n: i * 2 }] };
    }
    for i in 0..30 {
      th += Word { w: "w{i}", n: i * i };
      tr += Word { w: "word{i}", n: i };
    }
    for i in 0..36 { sp += Pt { x: i % 6, y: i / 6, label: "p{i}" }; }
    for i in 0..20 { so += Sv { k: i * 5, s: "s{i}" }; }
    bad += Bad { id: 1, names: ["x", "y"] };
    for i in 1..4 { sm += Small { id: i, v: i * 10 }; }
    ok = store_persist_copy(h, "{path}.h") && store_persist_copy(th, "{path}.th") && store_persist_copy(tr, "{path}.tr");
    ok = ok && store_persist_copy(sp, "{path}.sp") && store_persist_copy(so, "{path}.so");
    ok = ok && store_persist_copy(bad, "{path}.bad") && store_persist_copy(sm, "{path}.sm");
    if ok { println("written") } else { println("FAIL write") }
    return
  }
  got = store_load_key(h, "{path}.h", 9);
  r = h[9];
  tg = "";
  for t in r.tags? { tg += "{t},"; }
  it = "";
  for x in r.items? { it += "{x.tag}:{x.n},"; }
  println("key9 {got} {r.name} {tg} {r.sub.tag}:{r.sub.n} {it} len={len(h)} verify={store_verify(h)}");
  println("key10 {store_load_key(h, "{path}.h", 10)} len={len(h)}");
  h2: hash<Rec[id]> = [];
  n = store_load_keys(h2, "{path}.h", [0, 3, 4, 117]);
  println("keys {n} {h2[0].name} {h2[3].name} {h2[117].name} absent4={h2[4] == null} len={len(h2)} verify={store_verify(h2)}");
  println("text {store_load_key_text(th, "{path}.th", "w5")} {th["w5"].n} len={len(th)}");
  th2: hash<Word[w]> = [];
  n = store_load_keys_text(th2, "{path}.th", ["w1", "w2", "zz"]);
  println("texts {n} {th2["w2"].n} len={len(th2)} verify={store_verify(th2)}");
  println("trie {store_load_key_text(tr, "{path}.tr", "word12")} {tr["word12"].n} len={len(tr)}");
  tp: trie<Word[w]> = [];
  n = store_load_prefix(tp, "{path}.tr", "word1", 5);
  s = 0;
  for w in tp { s += w.n; }
  println("prefix5 {n} {s} len={len(tp)} verify={store_verify(tp)}");
  ta: trie<Word[w]> = [];
  n = store_load_prefix(ta, "{path}.tr", "word1", -1);
  s = 0;
  for w in ta { s += w.n; }
  println("prefix {n} {s} len={len(ta)}");
  n = store_load_box(sp, "{path}.sp", [3, 1], [1, 0], -1);
  s = 0;
  for p in sp { s += p.x + 10 * p.y; }
  println("box {n} {s} len={len(sp)} verify={store_verify(sp)}");
  sp4: spatial<Pt[x, y]> = [];
  println("box4 {store_load_box(sp4, "{path}.sp", [3, 1], [1, 0], 4)} len={len(sp4)}");
  n = store_load_range(so, "{path}.so", 10, 20);
  ks = "";
  for v in so { ks += "{v.k}={v.s},"; }
  println("range {n} {ks} len={len(so)} verify={store_verify(so)}");
  println("bad {store_load_key(bad, "{path}.bad", 1)} len={len(bad)}");
  println("small {store_load_key(sm, "{path}.sm", 2)} {sm[2].v} len={len(sm)} verify={store_verify(sm)}");
}
"#;

/// The paged half of the owner's acceptance (@PLN200 B5 step (e)): the images every backend
/// writes are read a page at a time by every backend — each loader, on every collection kind it
/// serves — and each reader answers the hand-computed values.  Then the C's images are the
/// interpreter's, byte for byte.  Id 9 is the fourth record (`i = 3`); the keys 0, 3 and 117 are
/// present and 4 is not; the trie's keys starting `word1` are `word1` and `word10`…`word19`, the
/// first five summing 1 + 10 + 11 + 12 + 13; the box from (3, 1) to (1, 0) holds x 1…3 by y 0…1;
/// the sorted keys run 0, 5, …, 95.
#[test]
fn the_paged_loaders_read_every_backends_image() {
    let Some(x) = Crossed::new("paged", PAGED_PROGRAM) else {
        return;
    };
    let want = "key9 true rec3 3,4, s3:-3 a3:3,b3:6, len=1 verify=true
key10 false len=1
keys 3 rec0 rec1 rec39 absent4=true len=3 verify=true
text true 25 len=1
texts 2 4 len=2 verify=true
trie true 12 len=1
prefix5 5 47 len=5 verify=true
prefix 11 146 len=11
box 6 42 len=6 verify=true
box4 4 len=4
range 3 10=s2,15=s3,20=s4, len=3 verify=true
bad false len=0
small true 20 len=1 verify=true
";
    for w in &x.backends {
        assert_eq!(
            x.run(w, "write", &x.dir().join(*w)),
            "written\n",
            "{w} writes"
        );
    }
    for w in &x.backends {
        for r in &x.backends {
            assert_eq!(
                x.run(r, "load", &x.dir().join(*w)),
                want,
                "{r} paging {w}'s images"
            );
        }
    }
    for suffix in ["h", "th", "tr", "sp", "so", "bad", "sm"] {
        let read = |w: &str| fa::read(x.dir().join(format!("{w}.{suffix}"))).expect("an image");
        assert_eq!(
            read("c"),
            read("interpret"),
            "the C's .{suffix} image is the interpreter's"
        );
    }
}
