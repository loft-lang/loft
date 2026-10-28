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

/// The inputs a unit names on its `/* link: … */` lines (an archive it is built with — a
/// `[c] shim`, the TLS client of a unit that fetches over a URL or pages an image), as
/// `c_driver::link_inputs` reads them.
fn unit_links(c_src: &std::path::Path) -> Vec<String> {
    fa::read_to_string(c_src)
        .expect("read the C")
        .lines()
        .filter_map(|l| l.strip_prefix("/* link: ")?.strip_suffix(" */"))
        .map(str::to_string)
        .collect()
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
    // A stdlib native the runtime has not ported is refused by its name, never written as C
    // that answers differently.
    // `host_output` is the browser host's channel, which the C runtime has not ported (@PLN203 C6).
    let p = parsed("fn up() -> integer { host_output(\"up\"); 1 }\n");
    let d = p.data.def_nr("n_up");
    let refusal = loft::c::emit_unit(&p.data, &p.database, &[d])
        .expect_err("a native without a C port is not carried");
    assert_eq!(refusal.function, "host_output");
    assert!(
        refusal.why.contains("native host_output"),
        "names the construct: {refusal}"
    );
}

/// @PLN203 C3 — the stdlib census: every native of `default/*.loft` has a C arm, except the
/// ones named here, each refused by name where a program reaches it.  A new native lands with
/// its C arm or with a row here saying which phase ports it.
#[test]
fn every_stdlib_native_has_a_c_arm_or_a_named_refusal() {
    let p = parsed("fn main() {}\n");
    let unported = loft::c::unported_natives(&p.data);
    let refused: &[&str] = &[
        // The browser host's channel (`--html`, `--native-wasm`): @PLN203 C6.
        "n_host_input",
        "n_host_output",
        // The test ledger's builtins (@PLN201 step 2): the C records no ledger yet.
        "n___ledger",
        "n___ledger_div",
        // A retired panic stub: the parser lowers a `par` loop to the queue natives.
        "n_parallel_for",
    ];
    let mut want: Vec<String> = refused.iter().map(|s| (*s).to_string()).collect();
    want.sort();
    assert_eq!(unported, want, "natives without a C arm");
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
        // The program is the macros under a bare `printf` main, not loft's runtime, so a
        // Windows stdout writes `\r\n`; what is compared is the arithmetic.
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
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

/// `@FR-R-RetAdopt` as C: a vector-returning function whose result local is delivered through
/// the hidden return buffer builds IN that buffer — the bind is the buffer, the witness store is
/// never minted, and the exit's clear-and-append copy emits nothing.  Four shapes adopt here:
/// the stdlib `split` (texts), a typed literal build (`vfill`), a struct build over a parameter
/// (`twice`, also nested in itself) and a recursion that walks its own call's result (`rec`);
/// the destinations are a fresh local, a record FIELD (a placed buffer), an inline argument and
/// a buffer reused by every round of a loop.  Each value is computed by hand, and the census
/// counts the live records after 10 and after 1010 rounds: an adopted bind that minted or kept
/// what the caller's buffer held would grow it.  `LOFT_NO_RETBUF_ADOPT=1` keeps the copies and
/// must answer the same — the falsifier of the structural count.
#[test]
fn a_returned_vector_builds_in_the_callers_buffer() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"
struct P { x: integer, y: integer }
struct Holder { name: text, items: vector<text>, pts: vector<P> }
fn vfill(n: integer) -> vector<text> {
  result: vector<text> = [];
  result += ["aa"];
  result += ["b{n}"];
  result
}
fn twice(v: vector<P>) -> vector<P> {
  out: vector<P> = [];
  for e in v {
    out += [P { x: e.x, y: e.y }];
    out += [P { x: e.x + 1000, y: e.y }];
  }
  out
}
fn rec(n: integer) -> vector<integer> {
  res = [];
  if n > 0 {
    for v in rec(n - 1) { res += [v + 100]; }
    res += [n];
  }
  res
}
fn census() -> integer {
  ct = store_memory();
  ci = ct.find("records ");
  if !ci { return -1 }
  cj = ci + 8;
  cn = 0;
  while cj < len(ct) && ct[cj] >= '0' && ct[cj] <= '9' {
    cn = cn * 10 + (ct[cj] as integer - '0' as integer);
    cj += 1;
  }
  cn
}
fn rounds(n: integer) -> integer {
  keep = "";
  ps: vector<P> = [P { x: 7, y: 8 }];
  for k in 0..n {
    v = "a,b,{k}".split(',');
    keep = v[2];
    ps = twice([P { x: k, y: 1 }]);
  }
  len(keep) + len(ps)
}
fn main() {
  parts = "a,bb,,ccc".split(',');
  println("{parts} {len("x,".split(','))} {len("".split(','))}");
  h = Holder { name: "h", items: [], pts: [] };
  h.items = vfill(3);
  println("{h.items}");
  h.items = "p;q".split(';');
  h.pts = twice([P { x: 1, y: 2 }, P { x: 3, y: 4 }]);
  println("{h.items} {len(h.pts)} {h.pts[1].x} {h.pts[3].y}");
  t = 0;
  s = "x";
  for k in 0..50 {
    v = s.split(',');
    t += len(v);
    s += ",y{k % 3}";
  }
  rounds(10);
  c10 = census();
  rounds(1000);
  println("census {census() - c10}");
  println("{t} {rec(4)} {twice(twice([P { x: 5, y: 6 }]))[3].x}");
}
"#;
    // "x,".split(',') ends a part at the separator and keeps the empty tail: 2 parts; "" has
    // none.  Round k of the loop splits k + 1 parts: 1 + 2 + … + 50 = 1275.  rec(n) is
    // rec(n - 1) each + 100, then n: [1], [101, 2], [201, 102, 3], [301, 202, 103, 4].
    // twice([5]) = [5, 1005]; twice of that = [5, 1005, 1005, 2005].
    let want = "[\"a\",\"bb\",\"\",\"ccc\"] 2 0\n[\"aa\",\"b3\"]\n[\"p\",\"q\"] 4 1001 4\n\
                census 0\n1275 [301,202,103,4] 2005\n";
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("ret-adopt");
    let file = dir.join("adopt.loft");
    fa::write(&file, src).expect("write program");
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
    // The adopted bind: a caller that offered no buffer gets one, an offered one is emptied.
    let bind = "___retbuf.rec == 0) {";
    for (mode, off, binds) in [("adopted", "0", 4), ("copied", "1", 0)] {
        let c = dir.join(format!("{mode}.c"));
        let emit = loft::platform::process::harness_command(loft)
            .env("LOFT_NO_RETBUF_ADOPT", off)
            .arg("--emit-c")
            .arg(&c)
            .arg(&file)
            .output()
            .expect("run loft --emit-c");
        assert!(
            emit.status.success(),
            "{mode}: the program is carried: {}",
            String::from_utf8_lossy(&emit.stderr)
        );
        let text = fa::read_to_string(&c).expect("read the C");
        assert_eq!(
            text.matches(bind).count(),
            binds,
            "{mode}: split, vfill, twice and rec each adopt their buffer"
        );
        let exe = dir.join(mode);
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
            "{mode}: cc rejected the C:\n{}",
            String::from_utf8_lossy(&cc.stderr)
        );
        let out = loft::platform::process::harness_command(&exe)
            .output()
            .expect("run the C program");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            want,
            "{mode}: the C program"
        );
    }
}

/// `@FR-F-Render`'s shortest form written without `printf` where the digits allow it: a float of
/// at most 15 significant digits (6 for a single) and magnitude in [1e-6, 1e15) is the integer
/// nearest |v| * 10^k for the least k that reads back (`loft_push_shortest_fast`); every other
/// float takes the `%.*e` search.  The spot values are Rust's `{}` by hand — across the fast
/// range, past its edges (17 digits, 1e21, 1e-7, the zeros) — and `bulk` prints 160 280 floats
/// and singles the C must render byte for byte as the interpreter does.
#[test]
fn a_float_prints_its_shortest_form_with_or_without_printf() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"fn bulk() {
  out = "";
  st = 12345;
  for i in 0..20000 {
    st = (st * 1103515245 + 12345) & 2147483647;
    a = i as float * 0.1;
    b = i as float / 3.0;
    c = 1.0 / (i + 1) as float;
    d = st as float * 0.00001;
    e = (st % 100000) as float / 1000.0 - 50.0;
    f = (st % 1000) as float * 1000000000.0;
    g = (i as float * 0.37) as single;
    h = (st % 10000) as single / 100.0f;
    out += "{a} {b} {c} {d} {e} {f} {g} {h}\n";
    if i % 2000 == 1999 { print(out); out = ""; }
  }
  for k in 0..40 {
    p = 1.0;
    for j in 0..k { p = p * 10.0; }
    print("{p} {1.0 / p} {p * 1.5} {p + 0.5} {0.1 + 0.2} {123456789012345.0} {999999999999999.9}\n");
  }
}

fn main() {
  println("{0.1 + 0.2} {1.0 / 3.0} {2.5} {100.0} {0.000001} {0.0000001} {123456789012345.0}");
  println("{1500000000000000.0} {0.37f} {1234.56} {-0.25} {1000000000000000000000.0} {-0.0} {0.0}");
  bulk();
}
"#;
    let spots = "0.30000000000000004 0.3333333333333333 2.5 100 0.000001 0.0000001 123456789012345\n\
                 1500000000000000 0.37 1234.56 -0.25 1000000000000000000000 -0 0\n";
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("shortest-float");
    let file = dir.join("ff.loft");
    fa::write(&file, src).expect("write program");
    let interp = loft::platform::process::harness_command(loft)
        .arg("--interpret")
        .arg(&file)
        .output()
        .expect("run loft");
    let want = String::from_utf8_lossy(&interp.stdout).to_string();
    assert!(want.starts_with(spots), "the interpreter's spot values");
    assert_eq!(want.lines().count(), 20042, "the interpreter's lines");
    let c = dir.join("ff.c");
    let _ = fa::remove_file(&c);
    let emit = loft::platform::process::harness_command(loft)
        .arg("--emit-c")
        .arg(&c)
        .arg(&file)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "{}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let exe = dir.join("ff");
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
        "{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .output()
        .expect("run the C program");
    let got = String::from_utf8_lossy(&out.stdout);
    assert!(
        got.starts_with(spots),
        "the C's spot values: {}",
        &got[..got.len().min(300)]
    );
    let first = got.lines().zip(want.lines()).position(|(g, w)| g != w);
    assert_eq!(first, None, "the first line the C renders otherwise");
    assert_eq!(got.lines().count(), want.lines().count(), "the C's lines");
}

/// `@FR-R-TypedKeyed` in the C: a hash looked up by one integer key, by two integer keys or by
/// one text key takes its typed entry, which compares the key in place.  A bucket's 32-bit tag
/// filters almost every other key first, so the comparison decides only on a TAG COLLISION —
/// these keys collide under `LOFT_HASH_SEED=12345` (found offline from the same SipHash-1-3):
/// `k0016413` and `k0039727`, 31541 and 69084, (612, 23) and (48, 51).  Each table holds the first
/// and is asked for both: the first is found, the second is not.  Values by hand.
#[test]
fn a_typed_hash_lookup_compares_the_key_on_a_tag_collision() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"
struct W { word: text, count: integer }
struct I { id: integer, v: integer }
struct P { q: integer, r: integer, v: integer }
struct Db { words: hash<W[word]>, ids: hash<I[id]>, pairs: hash<P[q, r]> }
fn main() {
  db = Db { words: [], ids: [], pairs: [] };
  db.words += [W { word: "k0016413", count: 7 }];
  db.ids += [I { id: 31541, v: 8 }];
  db.pairs += [P { q: 612, r: 23, v: 9 }];
  println("{db.words["k0016413"]?.count} {db.words["k0039727"] == null}");
  println("{db.ids[31541]?.v} {db.ids[69084] == null}");
  println("{db.pairs[612, 23]?.v} {db.pairs[48, 51] == null}");
}
"#;
    let want = "7 true\n8 true\n9 true\n";
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("typed-keyed");
    let file = dir.join("tk.loft");
    fa::write(&file, src).expect("write program");
    let interp = loft::platform::process::harness_command(loft)
        .env("LOFT_HASH_SEED", "12345")
        .arg("--interpret")
        .arg(&file)
        .output()
        .expect("run loft");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout),
        want,
        "the interpreter"
    );
    let c = dir.join("tk.c");
    let _ = fa::remove_file(&c);
    let emit = loft::platform::process::harness_command(loft)
        .arg("--emit-c")
        .arg(&c)
        .arg(&file)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "{}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let text = fa::read_to_string(&c).expect("read the C");
    for entry in ["loft_h_find_text(", "loft_h_find_i64(", "loft_h_find_ints("] {
        assert!(
            text.lines()
                .any(|l| l.contains(entry) && !l.starts_with("static")),
            "the typed entry {entry} is called"
        );
    }
    for (kind, flags) in [
        ("strict", &["-DLOFT_STRICT", "-pedantic", "-Werror"][..]),
        ("verify", &["-DLOFT_VERIFY", "-w"][..]),
    ] {
        let exe = dir.join(format!("tk-{kind}"));
        let cc = loft::platform::process::harness_command("cc")
            .args(["-std=c99", "-O1"])
            .args(flags)
            .arg(&c)
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .output()
            .expect("run cc");
        assert!(
            cc.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&cc.stderr)
        );
        let out = loft::platform::process::harness_command(&exe)
            .env("LOFT_HASH_SEED", "12345")
            .output()
            .expect("run the C program");
        assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{kind}");
    }
}

/// `@FR-R-ValueRecord` in the C, from native's plan: `mk` returns its `P` as a struct and takes
/// no buffer, its value local is read field by field and handed to a tuple parameter, and
/// copied into a vector slot it is materialised field by field; `pick` returns a `Q?` whose
/// null exit is the absent struct and whose presence word the `!= null` test reads; and a
/// build that writes its fields out of order runs them in the order the program wrote them.
/// Each value is computed by hand; `LOFT_NO_VALUE_RECORD=1` keeps the record form and answers
/// the same.
#[test]
fn a_record_result_travels_as_a_struct_in_the_c() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"struct P { x: integer, y: float }
struct Q { a: integer, b: integer }
struct Ord3 { f: integer, n: integer, ok: boolean }
fn mk(i: integer) -> P { P { x: i * 3, y: i as float * 0.5 } }
fn pick(i: integer) -> Q? { if i % 2 == 0 { Q { a: i, b: i + 1 } } else { null } }
fn sum_p(p: P) -> float { p.x as float + p.y }
fn tick(n: integer, log: &vector<integer>) -> integer { log += [n]; n }
fn ord(log: &vector<integer>) -> Ord3 { Ord3 { ok: tick(1, log) > 0, f: tick(2, log), n: tick(3, log) } }
fn main() {
  t = 0.0;
  c = 0;
  for i in 0..10 {
    p = mk(i);
    t += sum_p(p);
    q = pick(i);
    if q != null { c += q.a + q.b; }
  }
  v: vector<P> = [];
  for i in 0..4 { v += [mk(i)]; }
  sx = 0;
  sy = 0.0;
  for e in v { sx += e.x; sy += e.y; }
  log: vector<integer> = [];
  o = ord(log);
  println("{t} {c} {sx} {sy} {o.f} {o.n} {o.ok} {log}");
}
"#;
    // t: sum_p(mk(i)) = 3i + 0.5i over 0..10 = 3.5 * 45.  c: (i) + (i + 1) over the even i, 45.
    // v: x 0 3 6 9, y 0 0.5 1 1.5.  ord: the fields read in the order written, 1 2 3.
    let want = "157.5 45 18 3 2 3 true [1,2,3]\n";
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("value-record");
    let file = dir.join("vr.loft");
    fa::write(&file, src).expect("write program");
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
    let asan = {
        let probe = dir.join("probe.c");
        fa::write(&probe, "int main(void) { return 0; }\n").expect("write probe");
        loft::platform::process::harness_command("cc")
            .args(["-fsanitize=address", "-o"])
            .arg(dir.join("probe"))
            .arg(&probe)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    for (mode, off, structs) in [("value", "0", true), ("record", "1", false)] {
        let c = dir.join(format!("{mode}.c"));
        let _ = fa::remove_file(&c);
        let emit = loft::platform::process::harness_command(loft)
            .env("LOFT_NO_VALUE_RECORD", off)
            .arg("--emit-c")
            .arg(&c)
            .arg(&file)
            .output()
            .expect("run loft --emit-c");
        assert!(
            emit.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&emit.stderr)
        );
        let text = fa::read_to_string(&c).expect("read the C");
        for f in ["n_mk__fr(", "n_pick__fr("] {
            let sig = text
                .lines()
                .find(|l| l.starts_with("static ") && l.contains(f))
                .unwrap_or_default();
            assert_eq!(sig.starts_with("static loft_tup"), structs, "{mode}: {sig}");
        }
        let mut kinds: Vec<(&str, Vec<&str>)> = vec![
            ("strict", vec!["-DLOFT_STRICT", "-pedantic", "-Werror"]),
            ("verify", vec!["-DLOFT_VERIFY", "-w"]),
        ];
        if asan {
            kinds.push((
                "asan",
                vec![
                    "-g",
                    "-fsanitize=address,undefined",
                    "-fno-sanitize-recover=all",
                    "-w",
                ],
            ));
        }
        for (kind, flags) in kinds {
            let exe = dir.join(format!("{mode}-{kind}"));
            let cc = loft::platform::process::harness_command("cc")
                .args(["-std=c99", "-O1"])
                .args(&flags)
                .arg(&c)
                .arg("-o")
                .arg(&exe)
                .arg("-lm")
                .output()
                .expect("run cc");
            assert!(
                cc.status.success(),
                "{mode} {kind}: {}",
                String::from_utf8_lossy(&cc.stderr)
            );
            let out = loft::platform::process::harness_command(&exe)
                .output()
                .expect("run the C program");
            assert!(
                out.status.success(),
                "{mode} {kind}: exit {:?}\n{}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{mode} {kind}");
        }
    }
}

/// `@FR-R-SplitTable`: a `parts = text.split(c)` read only by its length, its element reads
/// and a walk keeps its pieces as slices of a copy of the text (`loft_stab`) and never builds
/// the vector or mints its buffer.  The cells: separators at both ends and doubled, an empty
/// text and one with no separator, an index from the end and one past it, a two-byte
/// separator, a walk, a rebind in a loop (the table refilled), and the source rebound after
/// the split — the pieces are the old text's, which a borrowed table would have read after it
/// was freed (AddressSanitizer).  Each value is computed by hand; `LOFT_NO_SPLIT_TABLE=1`
/// builds the vectors and answers the same.
#[test]
fn a_split_read_by_index_keeps_its_pieces_as_a_table() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"fn edges(src: text) -> text {
  parts = src.split(',');
  "{len(parts)}:{parts[0] ?? "N"}|{parts[1] ?? "N"}|{parts[len(parts) - 1] ?? "N"}|{parts[-1] ?? "N"}|{parts[99] ?? "N"}"
}
fn sizes(src: text) -> integer {
  parts = src.split('é');
  n = len(parts) * 1000;
  for i in 0..len(parts) { n += size(parts[i]?); }
  n
}
fn walked(src: text) -> text {
  parts = src.split(' ');
  out = "";
  for p in parts { out += "<{p}>"; }
  out
}
fn rebound(n: integer) -> integer {
  acc = 0;
  for k in 0..n {
    parts = "{k},{k * 2},x{k}".split(',');
    acc += len(parts) + size(parts[2]?) + size(parts[k % 3]?);
  }
  acc
}
fn changed() -> text {
  src = "a-bb-ccc";
  parts = src.split('-');
  src = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
  "{len(parts)} {parts[1]?} {parts[2]?} {len(src)}"
}
fn main() {
  println(edges(",a,,b,"));
  println(edges(""));
  println(edges("solo"));
  println("{sizes("aébcéédé")}");
  println(walked("one two  three"));
  println("{rebound(12)}");
  println(changed());
}
"#;
    // edges: ",a,,b," is five pieces, "" none, "solo" one; [-1] the last, [99] none.
    // sizes: five pieces of "aébcéédé" split on 'é', 1 + 2 + 0 + 1 + 0 bytes.
    // rebound: per k, 3 pieces + size of "x{k}" + size of piece k % 3: 6 6 7 6 6 7 6 7 7 6 8 9.
    let want = "5:|a|||N\n0:N|N|N|N|N\n1:solo|N|solo|solo|N\n5004\n<one><two><><three>\n81\n\
                3 bb ccc 50\n";
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("split-table");
    let file = dir.join("table.loft");
    fa::write(&file, src).expect("write program");
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
    let asan = {
        let probe = dir.join("probe.c");
        fa::write(&probe, "int main(void) { return 0; }\n").expect("write probe");
        loft::platform::process::harness_command("cc")
            .args(["-fsanitize=address", "-o"])
            .arg(dir.join("probe"))
            .arg(&probe)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    for (mode, off, tables) in [("tables", "0", 5), ("vectors", "1", 0)] {
        let c = dir.join(format!("{mode}.c"));
        let emit = loft::platform::process::harness_command(loft)
            .env("LOFT_NO_SPLIT_TABLE", off)
            .arg("--emit-c")
            .arg(&c)
            .arg(&file)
            .output()
            .expect("run loft --emit-c");
        assert!(
            emit.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&emit.stderr)
        );
        let text = fa::read_to_string(&c).expect("read the C");
        assert_eq!(
            text.lines()
                .filter(|l| l.contains("loft_stab_fill(&st"))
                .count(),
            tables,
            "{mode}: the tables filled"
        );
        let mut kinds: Vec<(&str, Vec<&str>)> = vec![
            ("strict", vec!["-DLOFT_STRICT", "-pedantic", "-Werror"]),
            ("verify", vec!["-DLOFT_VERIFY", "-w"]),
        ];
        if asan {
            kinds.push((
                "asan",
                vec![
                    "-g",
                    "-fsanitize=address,undefined",
                    "-fno-sanitize-recover=all",
                    "-w",
                ],
            ));
        }
        for (kind, flags) in kinds {
            let exe = dir.join(format!("{mode}-{kind}"));
            let cc = loft::platform::process::harness_command("cc")
                .args(["-std=c99", "-O1"])
                .args(&flags)
                .arg(&c)
                .arg("-o")
                .arg(&exe)
                .arg("-lm")
                .output()
                .expect("run cc");
            assert!(
                cc.status.success(),
                "{mode} {kind}: {}",
                String::from_utf8_lossy(&cc.stderr)
            );
            let out = loft::platform::process::harness_command(&exe)
                .output()
                .expect("run the C program");
            assert!(
                out.status.success(),
                "{mode} {kind}: exit {:?}\n{}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{mode} {kind}");
        }
    }
}

/// `@FR-R-Header` under the loop verdict native asks (`hoist::hoistable`): a loop holds the
/// headers of the vectors its admitted callees index and calls their `__inv` twin with them
/// (`@FR-R-Inputs`); a loop that grows a store keeps another vector's record and length and
/// derives its base again at each access (`loft_vh_fresh`, `@FR-R-Base`); and a loop that
/// pushes into the return buffer while a callee reads a parameter's vector is written twice,
/// the copy holding the parameter's headers running when their store numbers differ
/// (`@FR-R-Alias`'s versioned clause).  `grow` pushes 3000 elements into the store the held
/// `t.a` lives in, so its buffer moves under the held record: a base kept there reads freed
/// memory (AddressSanitizer) and fails the header proof (`LOFT_VERIFY`).  Each value is
/// computed by hand; each switch writes the plain form and answers the same.
#[test]
fn a_loop_holds_its_callees_headers_and_rederives_a_base_under_growth() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"
struct Two { a: vector<integer>, b: vector<integer> }
struct Sim { w: integer, v: vector<integer>, out: vector<integer> }
fn bump(src: vector<integer>, dst: &vector<integer>, i: integer) {
  if i < dst.len() { dst[i] = (dst[i] ?? 0) + (src[i] ?? 0) * 2; }
}
fn rounds(src: vector<integer>, dst: &vector<integer>) -> integer {
  acc = 0;
  for _r in 0..3 {
    for i in 0..len(src) { bump(src, dst, i); acc += dst[i] ?? 0; }
  }
  acc
}
fn grow(n: integer) -> integer {
  t = Two { a: [], b: [] };
  for k in 0..10 { t.a += [k + 1]; }
  for i in 0..n { t.b += [(t.a[i % len(t.a)] ?? 0) * 2 + len(t.a)]; }
  s = 0;
  for x in t.b { s += x; }
  s + len(t.b)
}
fn look(s: Sim, i: integer) -> integer {
  v = s.v;
  (v[i % s.w] ?? 0) * 3
}
fn build(s: Sim, n: integer) -> vector<integer> {
  pix: vector<integer> = [];
  for i in 0..n { pix += [look(s, i) + 1]; }
  pix
}
struct Pt { x: float, k: integer }
struct Pair { a: vector<Pt>, b: vector<Pt> }
fn mints(n: integer) -> text {
  m = Pair { a: [], b: [] };
  for i in 0..n {
    base = len(m.a);
    m.a += [Pt { x: i as float, k: base }];
    m.b += [Pt { x: 0.5, k: i * 2 }];
  }
  sa = 0;
  sx = 0.0;
  for p in m.a { sa += p.k; sx += p.x; }
  sb = 0;
  for p in m.b { sb += p.k; }
  "{len(m.a)} {len(m.b)} {sa} {sb} {sx}"
}
struct Cfg { k: integer, m: integer }
fn scal(c: Cfg, n: integer) -> integer {
  acc = 0;
  for i in 0..n {
    acc += c.k;
    if i % 3 == 0 { c.m += 1; }
    acc += c.m;
  }
  acc
}
fn main() {
  println("{scal(Cfg { k: 5, m: 0 }, 9)}");
  println(mints(1000));
  src: vector<integer> = [];
  dst: vector<integer> = [];
  for i in 0..8 { src += [i + 1]; dst += [1]; }
  println("{rounds(src, dst)} {dst}");
  println("{grow(3000)}");
  s = Sim { w: 5, v: [2, 4, 6, 8, 10], out: [] };
  p = build(s, 20);
  s.out = build(s, 7);
  println("{len(p)} {p[19]} {s.out}");
}
"#;
    // scal: c.k is read nine times (45) and held; c.m is written in the loop and read after
    // each write, 1 1 1 2 2 2 3 3 3 (18), so it is not held.
    // mints: two record vectors of one store minted in turn, each outgrowing its room many
    // times; a.k is the length read before each mint, 0..999 summing 499500 as a.x does, b.k
    // the doubled index, 999000.
    // rounds: after round r, dst[i] = 1 + 2r(i + 1); the reads sum 3·8 + 2·(1+2+3)·36 = 456.
    // grow: each pass of ten adds 2·55 + 10·10 = 210, 300 passes, plus 3000 elements.
    // build: v[i % 5]·3 + 1 cycles 7 13 19 25 31; p[19] is the fifth.
    let want = "63\n1000 1000 499500 999000 499500\n456 [7,13,19,25,31,37,43,49]\n66000\n\
                20 31 [7,13,19,25,31,7,13]\n";
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("loop-verdict");
    let file = dir.join("verdict.loft");
    fa::write(&file, src).expect("write program");
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
    let asan = {
        let probe = dir.join("probe.c");
        fa::write(&probe, "int main(void) { return 0; }\n").expect("write probe");
        loft::platform::process::harness_command("cc")
            .args(["-fsanitize=address", "-o"])
            .arg(dir.join("probe"))
            .arg(&probe)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    // (mode, switch, __inv calls, lines reading a fresh base, versioned loops, push headers,
    // `scal`'s held scalars)
    for (mode, switch, twins, fresh, versioned, pushed, scalars) in [
        ("held", "", 2, 3, 1, 2, 1),
        ("no-inputs", "LOFT_NO_CALLEE_INPUTS", 0, 3, 0, 2, 1),
        ("no-version", "LOFT_NO_DISTINCT_VERSION", 1, 3, 0, 2, 1),
        ("no-record-push", "LOFT_NO_RECORD_PUSH", 2, 3, 1, 0, 1),
        ("no-scalar", "LOFT_NO_SCALAR_HOIST", 2, 3, 1, 2, 0),
        ("unheld", "LOFT_NO_VECTOR_HOIST", 0, 0, 0, 0, 0),
    ] {
        let c = dir.join(format!("{mode}.c"));
        let mut emit = loft::platform::process::harness_command(loft);
        if !switch.is_empty() {
            emit.env(switch, "1");
        }
        let emit = emit
            .arg("--emit-c")
            .arg(&c)
            .arg(&file)
            .output()
            .expect("run loft --emit-c");
        assert!(
            emit.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&emit.stderr)
        );
        let text = fa::read_to_string(&c).expect("read the C");
        let body = &text[text.find("static void loft_drop_closure").unwrap_or(0)..];
        let calls = body
            .lines()
            .filter(|l| l.contains("__inv(") && !l.trim_start().starts_with("static"))
            .count();
        assert_eq!(calls, twins, "{mode}: the calls handed held headers");
        assert_eq!(
            body.lines()
                .filter(|l| l.contains("loft_vh_fresh(h"))
                .count(),
            fresh,
            "{mode}: the reads that derive their base again"
        );
        assert_eq!(
            body.matches("_pix.store_nr != ").count(),
            versioned,
            "{mode}: the loops written twice"
        );
        assert_eq!(
            body.lines()
                .filter(|l| l.contains(" = loft_ph_of("))
                .count(),
            pushed,
            "{mode}: the record push headers"
        );
        let scal = body
            .find("n_scal__fr(")
            .and_then(|at| body[at..].find("{\n").map(|o| at + o))
            .map_or("", |at| {
                &body[at..at + body[at..].find("\n}\n").unwrap_or(0)]
            });
        assert_eq!(
            scal.lines()
                .filter(|l| {
                    let t = l.trim_start();
                    t.starts_with('s')
                        && t[1..]
                            .split(" = ")
                            .next()
                            .is_some_and(|n| n.parse::<u32>().is_ok())
                })
                .count(),
            scalars,
            "{mode}: scal's held scalars"
        );
        let mut kinds: Vec<(&str, Vec<&str>)> = vec![
            ("strict", vec!["-DLOFT_STRICT", "-pedantic", "-Werror"]),
            ("verify", vec!["-DLOFT_VERIFY", "-w"]),
        ];
        if asan && mode == "held" {
            kinds.push((
                "asan",
                vec![
                    "-g",
                    "-fsanitize=address,undefined",
                    "-fno-sanitize-recover=all",
                    "-w",
                ],
            ));
        }
        for (kind, flags) in kinds {
            let exe = dir.join(format!("{mode}-{kind}"));
            let cc = loft::platform::process::harness_command("cc")
                .args(["-std=c99", "-O1"])
                .args(&flags)
                .arg(&c)
                .arg("-o")
                .arg(&exe)
                .arg("-lm")
                .output()
                .expect("run cc");
            assert!(
                cc.status.success(),
                "{mode} {kind}: {}",
                String::from_utf8_lossy(&cc.stderr)
            );
            let out = loft::platform::process::harness_command(&exe)
                .env("ASAN_OPTIONS", "detect_leaks=0")
                .output()
                .expect("run the C program");
            assert!(
                out.status.success(),
                "{mode} {kind}: exit {:?}\n{}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{mode} {kind}");
        }
    }
}

/// `@FR-R-PushFill`'s window clause as C: a counted push loop the Rust writer's own admission
/// passes (`hoist::push_loop` + `push_window_ok`, or `mint_loop` + `mint_window_ok` for record
/// mints) reserves its pushes once and pushes through a `loft_win` — a compare, a store and a
/// bump — whose length is written back at the loop's exit.  Every push kind is here (integer,
/// float, single, i32, u8, character, boolean, enum), a record mint whose literal writes every
/// field (no default image copied, `@FR-R-CompleteWrite`), a comprehension inside a
/// comprehension, and a window reopened on every pass of an outer loop (with the deep window
/// off, `LOFT_NO_DEEP_WINDOW=1`; by default that outer loop holds one deep window instead).
/// The counted rule declines where native's does: the enum vector (native's repeat fill), and
/// a mint under an `if` arm; the deep window (`hoist::deep_window_ok`) takes the enum loop.  Each value is computed by
/// hand; `LOFT_NO_PUSH_WINDOW=1` opens none and answers the same; the proofs are checked under
/// `LOFT_VERIFY`.  A window cannot serve a locked store, so its pushes take the growth arm,
/// the ordinary append, which refuses as the interpreter does.
#[test]
fn a_counted_push_loop_pushes_through_a_window() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"
enum Dir { North, East, South }
struct P { x: float, y: integer, name: text }
struct Q { x: float, id: integer }
fn ints(n: integer) -> integer {
  v: vector<integer> = [];
  for i in 0..n { v += [i * 3 + 1]; }
  sum(v) + len(v)
}
fn floats(n: integer) -> float {
  v: vector<float> = [];
  for i in 0..n { v += [i as float * 0.5]; }
  t = 0.0;
  for x in v { t += x; }
  t
}
fn singles(n: integer) -> integer {
  v: vector<single> = [];
  for i in 0..n { v += [i as single, 0.25f]; }
  round(v[len(v) - 2]? as float * 4.0) as integer + len(v)
}
fn narrow(n: integer) -> text {
  a: vector<i32> = [];
  b: vector<u8> = [];
  c: vector<character> = [];
  d: vector<boolean> = [];
  e: vector<Dir> = [];
  for i in 0..n { a += [(i - 2) as i32 ?? 0]; }
  for i in 0..n { b += [((i * 50) & 255) as u8]; }
  for i in 0..n { c += ['a']; }
  for i in 0..n { d += [i % 2 == 0]; }
  for i in 0..n { e += [Dir.East]; }
  "{a} {b} {len(c)}{c[0]} {d} {len(e)}"
}
fn records(n: integer) -> text {
  v: vector<P> = [];
  for i in 0..n {
    if i % 3 == 0 { v += [P { x: i as float, y: i * 10, name: "p{i}" }]; } else { v += [P { x: 0.5, y: i, name: "q" }]; }
  }
  "{len(v)} {v[3]?.name} {v[4]?.y} {v[n - 1]?.x}"
}
fn built(n: integer) -> integer {
  v: vector<Q> = [];
  for i in 0..n { v += [Q { x: i as float * 0.5, id: i * 7 }]; }
  t = 0;
  for q in v { t += q.id + round(q.x * 2.0) as integer; }
  t + len(v)
}
fn grid(side: integer) -> integer {
  g = [for y in 0..side { [for x in 0..side { x * y }] }];
  acc = 0;
  for r in g { acc += len(r) + sum(r); }
  acc
}
fn reopened(rounds: integer) -> integer {
  v: vector<integer> = [];
  for r in 0..rounds {
    for k in 0..4 { v += [r * 10 + k]; }
  }
  len(v) + v[len(v) - 1]?
}
fn main() {
  println("{ints(10)} {floats(6)} {singles(5)}");
  println("{narrow(4)}");
  println("{records(7)}");
  println("{grid(4)} {reopened(25)} {built(12)}");
}
"#;
    // ints: 3·45 + 10 + 10 = 155.  floats: 0.5·15.  singles: v[8] = 4.0 → 16, + 10 elements.
    // records: i = 3 is a `p` literal, i = 4 a `q` one, v[6].x = 6.  grid: each row y sums 6y
    // over 4 elements, 16 + 36.  reopened: 25·4 elements, the last 24·10 + 3.  built: the ids
    // 7·66, the halves doubled 66, 12 elements.
    let want = "155 7.5 26\n[-2,-1,0,1] [0,50,100,150] 4a [true,false,true,false] 4\n\
                7 p3 4 6\n52 343 540\n";
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("push-window");
    let file = dir.join("window.loft");
    fa::write(&file, src).expect("write program");
    let run_loft = |args: &[&str], f: &std::path::Path| {
        loft::platform::process::harness_command(loft)
            .args(args)
            .arg(f)
            .output()
            .expect("run loft")
    };
    let interp = run_loft(&["--interpret"], &file);
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout),
        want,
        "the interpreter"
    );
    let build = |c: &std::path::Path, exe: &std::path::Path, flags: &[&str]| {
        let cc = loft::platform::process::harness_command("cc")
            .args(["-std=c99", "-O1"])
            .args(flags)
            .arg(c)
            .arg("-o")
            .arg(exe)
            .arg("-lm")
            .output()
            .expect("run cc");
        assert!(
            cc.status.success(),
            "cc rejected {}:\n{}",
            c.display(),
            String::from_utf8_lossy(&cc.stderr)
        );
        loft::platform::process::harness_command(exe)
            .output()
            .expect("run the C program")
    };
    // `counted` keeps the deep window off, so the inner loop of `reopened` reopens its counted
    // window on every outer pass; by default the outer loop holds one deep window instead, and
    // `narrow`'s enum loop — native's repeat fill, which the C does not have — takes one too.
    for (mode, off, deep_off, windows, slots) in [
        ("windowed", "0", "0", 11, 1),
        ("counted", "0", "1", 10, 1),
        ("unwindowed", "1", "0", 0, 0),
    ] {
        let c = dir.join(format!("{mode}.c"));
        let emit = loft::platform::process::harness_command(loft)
            .env("LOFT_NO_PUSH_WINDOW", off)
            .env("LOFT_NO_DEEP_WINDOW", deep_off)
            .arg("--emit-c")
            .arg(&c)
            .arg(&file)
            .output()
            .expect("run loft --emit-c");
        assert!(
            emit.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&emit.stderr)
        );
        let text = fa::read_to_string(&c).expect("read the C");
        let opened = text
            .lines()
            .filter(|l| l.trim_start().starts_with("pw") && l.contains(" = loft_win_of(pv"))
            .count();
        assert_eq!(opened, windows, "{mode}: the windows opened");
        assert_eq!(
            text.matches("loft_win_slot(&pw").count(),
            slots,
            "{mode}: the record slots"
        );
        for (kind, flags) in [
            ("strict", &["-DLOFT_STRICT", "-pedantic", "-Werror"][..]),
            ("verify", &["-DLOFT_VERIFY", "-w"][..]),
        ] {
            let out = build(&c, &dir.join(format!("{mode}-{kind}")), flags);
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                want,
                "{mode} {kind}: the C program"
            );
        }
    }
    let locked = dir.join("locked.loft");
    fa::write(
        &locked,
        "fn build(n: integer) -> integer {\n  v: vector<integer> = [7];\n  v#lock = true;\n  \
         for i in 0..n { v += [i * 2]; }\n  len(v)\n}\nfn main() { println(\"{build(5)}\"); }\n",
    )
    .expect("write program");
    let interp = run_loft(&["--interpret"], &locked);
    let c = dir.join("locked.c");
    let emit = run_loft(&["--emit-c", c.to_str().expect("utf-8 path")], &locked);
    assert!(
        emit.status.success(),
        "{}",
        String::from_utf8_lossy(&emit.stderr)
    );
    assert!(
        fa::read_to_string(&c)
            .expect("read the C")
            .contains(" = loft_win_of(pv"),
        "the locked vector's loop is windowed"
    );
    let out = build(
        &c,
        &dir.join("locked"),
        &["-DLOFT_STRICT", "-pedantic", "-Werror"],
    );
    assert_eq!(
        out.status.code(),
        interp.status.code(),
        "the locked push's exit status"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("write to a locked store"),
        "the C refuses the locked push: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `vector_sum_int` as C (loop_kernels.rs, `vector::sum_blocks_i64`): the stdlib `sum` reads
/// its elements off the vector's base a block of 1024 at a time, and a block whose elements all
/// lie in [-2^40, 2^40) with the total more than 2^50 from the int64 edge is summed plainly; any
/// other block, and every element after it, takes the checked add.  Several blocks, an element
/// past the bound mid-vector, a null element, an overflow and a start near the edge each answer
/// what integer addition answers.  Dropping the block's bound test sums the null as a number.
#[test]
fn the_sum_kernel_sums_in_blocks_with_integer_rules() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"
fn main() {
  v: vector<integer> = [];
  for i in 0..3000 { v += [i]; }
  println("a {sum(v)}");
  w: vector<integer> = [];
  for i in 0..3000 { if i == 1500 { w += [2199023255552]; } else { w += [i]; } }
  println("b {sum(w)}");
  n: integer = null;
  x: vector<integer> = [1, 2];
  x += [n];
  x += [4];
  println("c {sum(x)}");
  o: vector<integer> = [9223372036854775807, 1];
  println("d {sum(o)}");
  println("e {sum([1, 2, 3], 9223372036854775800)}");
  m: vector<integer> = [];
  for i in 0..2050 { m += [0 - i * 1000000]; }
  println("f {sum(m)}");
}
"#;
    // a: 0 + 1 + … + 2999.  b: the same with 1500 replaced by 2^41.  c: a null element makes
    // the sum null.  d: i64::MAX + 1 overflows to null.  e: 9223372036854775800 + 6.
    // f: -10^6 · (0 + … + 2049).
    let want =
        "a 4498500\nb 2199027752552\nc null\nd null\ne 9223372036854775806\nf -2100225000000\n";
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("sum-kernel");
    let file = dir.join("sum.loft");
    fa::write(&file, src).expect("write program");
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
    let c = dir.join("sum.c");
    let emit = loft::platform::process::harness_command(loft)
        .arg("--emit-c")
        .arg(&c)
        .arg(&file)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "{}",
        String::from_utf8_lossy(&emit.stderr)
    );
    for (mode, flags) in [
        ("strict", &["-DLOFT_STRICT", "-pedantic", "-Werror"][..]),
        ("verify", &["-DLOFT_VERIFY", "-w"][..]),
    ] {
        let exe = dir.join(mode);
        let cc = loft::platform::process::harness_command("cc")
            .args(["-std=c99", "-O1"])
            .args(flags)
            .arg(&c)
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .output()
            .expect("run cc");
        assert!(
            cc.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&cc.stderr)
        );
        let out = loft::platform::process::harness_command(&exe)
            .output()
            .expect("run");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            want,
            "{mode}: the C program"
        );
    }
}

/// `@C156` (`formal/formatting.md` `(F-Float-Text)`) — every conversion between a float and text
/// rounds correctly, an exact tie to the even digit, on the interpreter, native and the C alike.
/// The single 1549.40625 lies exactly between 1549.4062 and 1549.4063 (the even one is taken,
/// where `core::fmt` alone gave the odd); `.P` rounds the value held, a tie to even (`2.675` is
/// held as 2.67499…); a negative value that rounds to zero keeps its sign; 2^89 is a power of
/// two whose nearest 16-digit candidate does not read back and whose neighbour does (the gap
/// below it is the narrower), so the shortest form is that neighbour and not a longer string;
/// and the parse forms read as the nearest float, an unreadable text as null.
#[test]
fn float_text_rounds_ties_to_even_on_every_path() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"
fn main() {
  s: single = 1549.40625f;
  println("{s} {-s}");
  println("{0.125:.2} {2.5:.0} {3.5:.0} {2.675:.2} {-0.0004:.2} {-0.0:.1}");
  x = 1.0;
  for i in 0..89 { x = x * 2.0; }
  println("{x}");
  println("{("2.5" as float?) ?? 0.0} {(".5" as float?) ?? 0.0} {("5." as float?) ?? 0.0} {("1e23" as float?) ?? 0.0} {("x" as float?) ?? -1.0}");
}
"#;
    let want = "1549.4062 -1549.4062\n0.12 2 4 2.67 -0.00 -0.0\n618970019642690200000000000\n\
                2.5 0.5 5 100000000000000000000000 -1\n";
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("float-text");
    let file = dir.join("float.loft");
    fa::write(&file, src).expect("write program");
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
    let c = dir.join("float.c");
    let emit = loft::platform::process::harness_command(loft)
        .arg("--emit-c")
        .arg(&c)
        .arg(&file)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "{}",
        String::from_utf8_lossy(&emit.stderr)
    );
    for (mode, flags) in [
        ("strict", &["-DLOFT_STRICT", "-pedantic", "-Werror"][..]),
        ("verify", &["-DLOFT_VERIFY", "-w"][..]),
    ] {
        let exe = dir.join(mode);
        let cc = loft::platform::process::harness_command("cc")
            .args(["-std=c99", "-O1"])
            .args(flags)
            .arg(&c)
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .output()
            .expect("run cc");
        assert!(
            cc.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&cc.stderr)
        );
        let out = loft::platform::process::harness_command(&exe)
            .output()
            .expect("run");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            want,
            "{mode}: the C program"
        );
    }
}

/// The text rewrites as C, each read from the analysis native uses: a `for p in t.split(c)` loop
/// takes its pieces from the text with no vector built (`@FR-R-LazySplit`, `hoist::lazy_splits`
/// — none for an empty or null text, the rest after the last separator, a multi-byte separator,
/// a source the loop body writes walked as a copy, a nested split, a `break`); a walked element
/// or parameter text is read as a view, not copied (`@FR-R-TextBorrow`); and `for c in t` takes
/// an ASCII byte without decoding it (`@FR-R-CharWalk`).  Each line is the interpreter's own
/// answer, and the C must print it in the strict and the verify builds.
#[test]
fn the_text_walks_answer_as_the_interpreter_does() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"
fn pieces(t: text, label: text) {
  out = "";
  n = 0;
  for p in t.split(',') { out += "[{p}]"; n += 1; }
  println("{label}: {n} {out}");
}
fn multi(t: text) {
  a = ""; for p in t.split('é') { a += "<{p}>"; }
  b = ""; for p in t.split('€') { b += "<{p}>"; }
  c = ""; for p in t.split('😀') { c += "<{p}>"; }
  println("{a} {b} {c}");
}
fn walk(t: text) -> text {
  r = "";
  for c in t { if c >= 'a' && c <= 'z' { r += "{c}"; } else { r += "_"; } }
  r
}
fn main() {
  pieces("", "empty");
  pieces("a", "one");
  pieces("a,", "trailing");
  pieces(",a", "leading");
  pieces("a,,b", "double");
  pieces(",,,", "only");
  n: text = null;
  pieces(n, "null");
  multi("xéy€z😀w");
  multi("é€😀");
  s = "p,q,r";
  got = "";
  for p in s.split(',') { s += ",x"; got += p; }
  println("written: {got} {s}");
  nest = "";
  for row in "a b,c d".split(',') { for w in row.split(' ') { nest += "({w})"; } }
  println("nested: {nest}");
  firsts = "";
  for p in "1,2,3,4".split(',') { if p == "3" { break; } firsts += p; }
  println("break: {firsts}");
  println("walk: {walk("héllo wörld ABC xyz")}");
  words: vector<text> = ["alpha", "βeta", "", "gamma"];
  total = 0;
  for w in words { total += len(w) * 10 + size(w); }
  words += ["late"];
  println("borrow: {total} {len(words)} {words.join("|")}");
}
"#;
    let want = "empty: 0 \none: 1 [a]\ntrailing: 2 [a][]\nleading: 2 [][a]\ndouble: 3 [a][][b]\nonly: 4 [][][][]\nnull: 0 \n<x><y€z😀w> <xéy><z😀w> <xéy€z><w>\n<><€😀> <é><😀> <é€><>\nwritten: pqr p,q,r,x,x,x\nnested: (a)(b)(c)(d)\nbreak: 12\nwalk: h_llo_w_rld_____xyz\nborrow: 155 5 alpha|βeta||gamma|late\n";
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("text-walks");
    let file = dir.join("walks.loft");
    fa::write(&file, src).expect("write program");
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
    let c = dir.join("walks.c");
    let emit = loft::platform::process::harness_command(loft)
        .arg("--emit-c")
        .arg(&c)
        .arg(&file)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "{}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let text = fa::read_to_string(&c).expect("read the C");
    assert!(
        text.matches(" = loft_ls_of(").count() >= 8,
        "every split loop is lazy"
    );
    for (mode, flags) in [
        ("strict", &["-DLOFT_STRICT", "-pedantic", "-Werror"][..]),
        ("verify", &["-DLOFT_VERIFY", "-w"][..]),
    ] {
        let exe = dir.join(mode);
        let cc = loft::platform::process::harness_command("cc")
            .args(["-std=c99", "-O1"])
            .args(flags)
            .arg(&c)
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .output()
            .expect("run cc");
        assert!(
            cc.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&cc.stderr)
        );
        let out = loft::platform::process::harness_command(&exe)
            .output()
            .expect("run");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            want,
            "{mode}: the C program"
        );
    }
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
        include_str!("../src/c/loft_text.h"),
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
        include_str!("../src/c/loft_text.h"),
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
        include_str!("../src/c/loft_text.h"),
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

/* `n` is read through its address, after `r`'s call wrote it: C leaves the order the arguments
 * of one call are evaluated in unspecified, and x86-64 gcc read a stale count first. */
static void ids(const char *tag, uint32_t *r, const uint32_t *n) {
    printf("%s", tag);
    for (uint32_t i = 0; i < *n; i++) printf(" %u", r[i]);
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
            ids("n", loft_rx_near(pts, K_AXES, 2, from, lim, &n), &n);
        } else if (op == 'B') {
            if (fscanf(f, "%lld %lld %lld %lld", &x, &y, &tx, &ty) != 4) return 2;
            int64_t from[2] = { x, y }, till[2] = { tx, ty };
            ids("b", loft_rx_box(pts, K_AXES, 2, from, till, SIZE_MAX, &n), &n);
        } else if (op == 'X') {
            if (fscanf(f, "%63s", buf) != 1) return 2;
            ids("x", loft_rx_prefix(words, K_WORD, loft_lit(buf, strlen(buf)), SIZE_MAX, &n), &n);
        }
    }
    ids("p", loft_rx_records(pts, &n), &n);
    ids("t", loft_rx_records(words, &n), &n);
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
            .args(unit_links(&c_src))
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

/// One parse's answer in the form both sides print: the value tree, or the error's offset,
/// JSON pointer and message.
fn json_dump(r: &Result<loft::json::Parsed, loft::json::ParseError>) -> String {
    use loft::json::Parsed;
    fn hex(s: &str) -> String {
        s.bytes().map(|b| format!("{b:02x}")).collect()
    }
    fn val(p: &Parsed, out: &mut String) {
        match p {
            Parsed::Null => out.push('n'),
            Parsed::Bool(b) => out.push_str(if *b { "b1" } else { "b0" }),
            Parsed::Number(n) => {
                let _ = write!(out, "N{:016x}", n.to_bits());
            }
            Parsed::Int(i) => {
                let _ = write!(out, "I{i}");
            }
            Parsed::Str(s) => {
                let _ = write!(out, "S{}", hex(s));
            }
            Parsed::Ident(s) => {
                let _ = write!(out, "D{}", hex(s));
            }
            Parsed::Array(items) => {
                out.push('[');
                for (i, x) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    val(x, out);
                }
                out.push(']');
            }
            Parsed::Object(fields) => {
                out.push('{');
                for (i, (k, off, x)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    let _ = write!(out, "{}@{off}=", hex(k));
                    val(x, out);
                }
                out.push('}');
            }
            Parsed::Constructor(tag, off, body) => {
                let _ = write!(out, "C{}@{off}(", hex(tag));
                val(body, out);
                out.push(')');
            }
        }
    }
    match r {
        Ok(p) => {
            let mut s = String::new();
            val(p, &mut s);
            s
        }
        Err(e) => format!("E{}|{}|{}", e.byte_offset, hex(&e.path), hex(&e.message)),
    }
}

/// The texts the JSON differential reads: valid documents of every shape, loft literals, and
/// each of them cut short, with a character dropped, replaced and inserted — the mutations
/// reach the escapes, the number grammar and every delimiter's error.
fn json_inputs() -> Vec<String> {
    let seeds = [
        r#"{"a": 1, "b": [true, false, null], "c": {"d": "e"}}"#,
        r#"[1, -2, 3.5, -0.25e3, 1E+9, 0, -0, 12345678901234567890]"#,
        r#""plain""#,
        r#""esc \" \\ \/ \b \f \n \r \t \u00e9 \ud83d\ude00 \ud800x \udc00 \ud800\u0041""#,
        r#"{"k": "\u12", "q": "\x41"}"#,
        "[ \"tab\there\", \"nl\nx\" ]",
        "{name: \"x\", v: 7, tag: Color.Red, e: Point{x: 1, y: 2}, w: Daily}",
        "[Red, Green{v: 1}, null, true]",
        "  \r\n {\"x\" :\t[ ] , \"y\" : { } }  ",
        "#!/usr/bin/x\n[1, 2]",
        "\u{feff}{\"bom\": 1}",
        "[1, 2,]",
        "{\"a\" 1}",
        "{\"a\": 1,}",
        "[007, 0x1f, 1_0, 1., .5, -, - 5, 1e, 1e+]",
        "{\"é\": \"ünï\", 'c': 1}",
        "[1..2, 1.a, 0b101, 0o17, 0xFFz, 1_000_0, 1__0, 2.5f, 2.5q, 1e5x, 12abc]",
        r#"["\u{41}\u{1F600}\u{}\u{110000}", "\x7f\x80\xg", "\0\'\`", "\ud83d😀", "\ud83d\"x"]"#,
        r#"{'a': 'b\'c', 'd': '', 'A': 1, k: 'x\ud800y'}"#,
        r#"{"deep": [[[{"x": [1, {"y": [true, "é"]}]}]]], "z": -0.0e-0}"#,
        "nul",
        "truex",
        "@",
        "",
    ];
    let alphabet = [
        "\"", "\\", "u", "{", "}", "[", "]", ",", ":", "-", ".", "e", "0", "9", "x", "_", " ",
        "\n", "\t", "é", "'", "n", "t", "f", "/", "d", "8", "a",
    ];
    let mut out: Vec<String> = Vec::new();
    for s in seeds {
        out.push(s.to_string());
        let cuts: Vec<usize> = (0..=s.len()).filter(|&i| s.is_char_boundary(i)).collect();
        for &i in &cuts {
            out.push(s[..i].to_string());
        }
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            out.push(format!("{}{}", &s[..a], &s[b..]));
            for c in alphabet {
                out.push(format!("{}{c}{}", &s[..a], &s[b..]));
                out.push(format!("{}{c}{}", &s[..a], &s[a..]));
            }
        }
    }
    out.retain(|s| !s.contains('`'));
    out.sort();
    out.dedup();
    out
}

/// The C reads JSON and loft literals as `json::parse_with` reads them (`loft_json.h`, a port
/// of `JParser` over loft's JSON-mode lexer): over every input, in both dialects, the same
/// value tree, or the same error at the same offset with the same JSON pointer.  The inputs
/// are documents and their mutations (`json_inputs`); a backtick is left out, where the lexer
/// opens a loft text block the port does not read.
#[test]
fn the_c_reads_json_as_json_rs_does() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let inputs = json_inputs();
    let mut want = String::new();
    for lenient in [false, true] {
        let dialect = if lenient {
            loft::json::Dialect::Lenient
        } else {
            loft::json::Dialect::Strict
        };
        for s in &inputs {
            let r = std::panic::catch_unwind(|| loft::json::parse_with(s, dialect));
            let Ok(r) = r else {
                panic!("json.rs panicked reading {s:?} ({dialect:?})");
            };
            let _ = writeln!(want, "{}", json_dump(&r));
        }
    }
    let mut c = String::from("#define LOFT_JSON 1\n");
    for part in [
        include_str!("../src/c/loft_cc.h"),
        include_str!("../src/c/loft_text.h"),
        include_str!("../src/c/loft_store.h"),
        include_str!("../src/c/loft_rt.h"),
        include_str!("../src/c/loft_json.h"),
    ] {
        c.push_str(part);
    }
    c.push_str(
        r#"
static void loft_init(void) {}

static void hex(const char *p, size_t n) {
    for (size_t i = 0; i < n; i++) printf("%02x", (unsigned char) p[i]);
}

static void dump(const loft_jv *v) {
    switch (v->kind) {
    case LOFT_JV_NULL: printf("n"); return;
    case LOFT_JV_BOOL: printf(v->b ? "b1" : "b0"); return;
    case LOFT_JV_NUMBER: { uint64_t u; memcpy(&u, &v->n, 8); printf("N%016llx", (unsigned long long) u); return; }
    case LOFT_JV_INT: printf("I%lld", (long long) v->i); return;
    case LOFT_JV_STR: printf("S"); hex(v->s.p, v->s.len); return;
    case LOFT_JV_IDENT: printf("D"); hex(v->s.p, v->s.len); return;
    case LOFT_JV_ARRAY:
        printf("[");
        for (size_t i = 0; i < v->nkids; i++) { if (i) printf(","); dump(&v->kids[i]); }
        printf("]");
        return;
    case LOFT_JV_OBJECT:
        printf("{");
        for (size_t i = 0; i < v->nkids; i++) {
            if (i) printf(",");
            hex(v->keys[i].p, v->keys[i].len);
            printf("@%zu=", v->key_offs[i]);
            dump(&v->kids[i]);
        }
        printf("}");
        return;
    case LOFT_JV_CTOR:
        printf("C"); hex(v->s.p, v->s.len); printf("@%zu(", v->off); dump(&v->kids[0]); printf(")");
        return;
    }
}

int main(int argc, char **argv) {
    FILE *f = fopen(argv[1], "rb");
    if (!f) return 2;
    static char buf[1 << 16];
    size_t n;
    for (int lenient = 0; lenient < 2; lenient++) {
        rewind(f);
        /* The length, exactly one newline, then the bytes: `%zu\n` would eat an input's
         * leading whitespace. */
        while (fscanf(f, "%zu", &n) == 1) {
            if (fgetc(f) != '\n') return 4;
            if (n > 0 && fread(buf, 1, n, f) != n) return 3;
            loft_jv v;
            loft_jfault e;
            loft_text path = { 0, 0, 0 };
            if (loft_json_parse((const uint8_t *) buf, n, lenient, &v, &e, &path)) {
                dump(&v);
            } else {
                printf("E%zu|", e.at);
                hex(path.p ? path.p : "", path.len);
                printf("|");
                hex(e.msg, strlen(e.msg));
            }
            printf("\n");
            loft_jv_free(&v);
            loft_text_free(&path);
        }
    }
    (void) argc;
    return 0;
}
"#,
    );
    let dir = scratch("json");
    let (src, exe, data) = (dir.join("json.c"), dir.join("json"), dir.join("inputs"));
    fa::write(&src, &c).expect("write C");
    let mut bytes = Vec::new();
    for s in &inputs {
        bytes.extend_from_slice(format!("{}\n", s.len()).as_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }
    fa::write(&data, &bytes).expect("write inputs");
    let cc = loft::platform::process::harness_command("cc")
        .args(["-std=c99", "-O1", "-Wno-unused-function"])
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .arg("-lm")
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "cc rejected loft_json.h:\n{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .arg(&data)
        .output()
        .expect("run the json program");
    let got = String::from_utf8_lossy(&out.stdout);
    let n = inputs.len();
    let mut bad = Vec::new();
    for (i, (w, g)) in want.lines().zip(got.lines()).enumerate() {
        if w != g {
            bad.push(format!(
                "{} {:?}\n    json.rs {w}\n    the C   {g}",
                if i < n { "strict " } else { "lenient" },
                inputs[i % n]
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} reads differ, the first:\n{}",
        bad.len(),
        2 * n,
        bad.iter().take(12).cloned().collect::<Vec<_>>().join("\n")
    );
    assert_eq!(got.lines().count(), 2 * n, "the C stopped early");
}

/// The types `T.parse` is crossed with (`the_c_parses_text_as_the_interpreter_does`): every
/// scalar, every narrow width, nullable and declared-default fields, a plain and a struct enum,
/// a nested record, vectors of scalars and records — and a non-null enum without a default,
/// which the checked `text as E2?` answers null for when the text leaves it out.
const PARSE_TYPES: &str = r##"enum Color { Red, Green, Blue }
enum Shape { Circle { radius: float }, Square { side: integer = 4 }, Dot }
struct Inner { p: integer, q: text = "dq" }
struct E2 { m: Color, a: integer }
struct J2 { a: integer, c: text, d: boolean, e: character, f: u8, g: i8?, n: Inner, o: vector<integer>, r: vector<Inner>,
  x: JsonValue, v: vector<JsonValue>, u: single, y: float = 2.5, t: integer?, w: vector<text> }
struct Rec { a: integer, b: float, c: text, d: boolean, e: character, f: u8, g: i8, h: u16, i: i16, j: i32, k: u32,
  l: integer = 5, m: Color = Blue, n: Inner, o: vector<integer>, r: vector<Inner>, s: Shape = Dot, t: integer?, u: single,
  v: text?, w: u8?, x: boolean?, y: float = 2.5, z: Color? }
fn check(t: text) {
  println("#check");
  r = Rec.parse(t);
  println("{r} {r#errors} [{json_errors()}]");
  w = vector<Inner>.parse(t);
  println("{w} {w#errors}");
  e = t as E2?;
  println("{e} [{json_errors()}]");
  q = J2.parse(json_parse(t));
  println("{q.a} {q.c} {q.d} {q.e} {q.f} {q.g} {q.n} {q.o} {q.r} {q.u} {q.y} {q.t} {q.w} {q.x.to_json()} {q#errors} [{json_errors()}]");
  for jv in q.v { print("{jv.to_json()};"); }
  println("");
}
"##;

/// The documents: seeds that reach every field kind, a misfit of each, a malformed document,
/// the tagged and qualified spellings, and every truncation, deletion and replacement of them.
fn parse_inputs() -> Vec<String> {
    let seeds = [
        r#"{"a":1,"b":2.5,"c":"x","d":true,"e":"Q","f":200,"g":-100,"h":60000,"i":-30000,"j":70000,"k":4000000000,"l":3,"m":Blue,"n":{"p":1,"q":"w"},"o":[1,2,3],"r":[{"p":4,"q":"z"}],"s":Circle{radius:2.0},"t":null,"u":1.25}"#,
        r#"{a:7, m: Color.Red, s: Square{side: 3}, o: [], r: [{p: 1}, null, {q: "k"}], n: null}"#,
        r#"{"s":"Dot","m":"Green","e":65,"c":null,"b":"bad","d":1}"#,
        "Rec{a: 1, k: -1, f: 256, h: 70000, i: 40000, l: 9, t: 5}",
        r#"{"o":[1,"x",3],"r":[{"p":"no"},{"p":2}],"n":{"q":1,"zz":[1,{"y":2}]}}"#,
        r#"{"a":-9223372036854775808,"u":1e400,"b":-0.0,"g":127,"l":-1,"c":"é\n"}"#,
        r#"[{"p":5},{"q":"r"}]"#,
        r#"{"s":{"Circle":{"radius":1}},"z":Purple,"x":false}"#,
        r#"{"a":3.7,"c":5,"d":"t","e":"xy","f":300,"g":null,"x":{"k":[1,{"m":"n"}]},"v":["s",[2],{"o":null},7,true],"w":["a",1,null]}"#,
        r#"{"e":97,"o":[1.9,"q",null],"r":[{"p":"z"},7],"x":"str","n":{"p":2.5,"q":["no"]},"y":null,"u":2}"#,
    ];
    let alphabet = ["{", "}", "[", "]", ":", ",", "\"", "1", "null", "Blue"];
    let mut out: Vec<String> = Vec::new();
    for s in seeds {
        let cuts: Vec<usize> = (0..=s.len()).filter(|&i| s.is_char_boundary(i)).collect();
        for &i in &cuts {
            out.push(s[..i].to_string());
        }
        for (k, w) in cuts.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            out.push(format!("{}{}", &s[..a], &s[b..]));
            out.push(format!(
                "{}{}{}",
                &s[..a],
                alphabet[k % alphabet.len()],
                &s[b..]
            ));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `T.parse(text)` in the C (`loft_parse.h`, a port of `record_from_text` and its walk)
/// answers what the interpreter answers: over every document, the record, its `#errors` and
/// `json_errors()`, for a record, a vector of records, the checked cast, and the two-stage
/// `T.parse(json_parse(text))` (`struct_from_jsonvalue`).  Two seeds'
/// answers are pinned as hand-checked values, so an empty run cannot pass.
#[test]
fn the_c_parses_text_as_the_interpreter_does() {
    let inputs = parse_inputs();
    let mut program = PARSE_TYPES.to_string();
    program.push_str("fn main() {\n");
    for d in &inputs {
        let mut lit = String::new();
        for ch in d.chars() {
            match ch {
                '{' => lit.push_str("{{"),
                '}' => lit.push_str("}}"),
                '"' => lit.push_str("\\\""),
                '\\' => lit.push_str("\\\\"),
                '\n' => lit.push_str("\\n"),
                c => lit.push(c),
            }
        }
        program.push_str(&format!("  check(\"{lit}\");\n"));
    }
    program.push_str("}\n");
    let Some(x) = Crossed::new("parse", &program) else {
        return;
    };
    let path = x.dir().join("unused");
    let want = x.run("interpret", "", &path);
    let got = x.run("c", "", &path);
    // Each check's answers follow its `#check` line: a text can hold a line break.
    let split =
        |out: &str| -> Vec<String> { out.split("#check\n").skip(1).map(str::to_string).collect() };
    let (want, got) = (split(&want), split(&got));
    assert_eq!(want.len(), inputs.len(), "the interpreter ran every check");
    for pinned in [
        r#"{a:7,b:0,c:"",d:false,e:null,f:0,g:-128,h:0,i:-32768,j:0,k:0,l:5,m:Red,n:{p:0,q:"dq"},o:[],r:[{p:1,q:"dq"},{p:0,q:"dq"},{p:0,q:"k"}],s:Square {side:3},t:null,u:0,v:null,w:null,x:null,y:2.5,z:null} [] []"#,
        r#"{a:0,b:0,c:"",d:false,e:'A',f:0,g:-128,h:0,i:-32768,j:0,k:0,l:5,m:Green,n:{p:0,q:"dq"},o:[],r:[],s:Dot {},t:null,u:0,v:null,w:null,x:null,y:2.5,z:null} ["line 1:40 path:b","line 1:50 path:d"] [line 1:40 path:b|line 1:50 path:d]"#,
    ] {
        assert!(
            want.iter().any(|w| w.lines().next() == Some(pinned)),
            "the interpreter answers {pinned}"
        );
    }
    let mut bad = Vec::new();
    for (i, (w, g)) in want.iter().zip(&got).enumerate() {
        if w != g {
            bad.push(format!(
                "{:?}\n    interpreter {w}\n    the C       {g}",
                inputs[i]
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} checks differ, the first:\n{}",
        bad.len(),
        inputs.len(),
        bad.iter().take(8).cloned().collect::<Vec<_>>().join("\n")
    );
    assert_eq!(got.len(), want.len(), "the C stopped early");
}

/// The C is strict C99 where it uses the math constants (`PI`, `E` — `M_PI` and `M_E` are
/// POSIX and `-std=c99` declares neither), and the release-pass probe
/// (`LOFT_RELEASE_PASS_PROBE=1`, @PLN201) gives the C generator the ship tier's forms: an
/// overflowing product the checked C answers as null wraps — DEFINED, through unsigned
/// arithmetic — while a zero divisor stays null and a value that never faulted is the same.
/// Values hand-computed: 3037000500² = 9223372037000250000, which is 2⁶⁴ −
/// 9223372036709301616 past `i64::MAX`.
#[test]
fn the_c_is_strict_c99_and_the_probe_wraps_where_the_checked_c_answers_null() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let dir = scratch("probe");
    let file = dir.join("probe.loft");
    fa::write(
        &file,
        "fn big(n: integer) -> integer { n * n }\n\
         fn d(a: integer, b: integer) -> integer { a / b }\n\
         fn neg(a: integer) -> integer { -a }\n\
         fn main() {\n\
           println(\"{PI > 3.141592653589 and PI < 3.141592653590} {E > 2.718281828459 and E < 2.718281828460}\");\n\
           println(\"{big(3037000500)} {big(1000)} {d(7, 2)} {d(7, 0)} {neg(5)}\");\n\
         }\n",
    )
    .expect("write program");
    let loft = env!("CARGO_BIN_EXE_loft");
    let run = |probe: bool| -> String {
        let c = dir.join(if probe { "ship.c" } else { "checked.c" });
        let mut emit = loft::platform::process::harness_command(loft);
        emit.arg("--emit-c").arg(&c).arg(&file);
        if probe {
            emit.env("LOFT_RELEASE_PASS_PROBE", "1");
        } else {
            emit.env_remove("LOFT_RELEASE_PASS_PROBE");
        }
        let emitted = emit.output().expect("run loft --emit-c");
        assert!(
            emitted.status.success(),
            "the program is carried: {}",
            String::from_utf8_lossy(&emitted.stderr)
        );
        let exe = dir.join(if probe { "ship" } else { "checked" });
        let cc = loft::platform::process::harness_command("cc")
            .args(["-std=c99", "-O2", "-DLOFT_STRICT", "-pedantic", "-Werror"])
            .arg(&c)
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .output()
            .expect("run cc");
        assert!(
            cc.status.success(),
            "cc rejected the {} C:\n{}",
            if probe { "probed" } else { "checked" },
            String::from_utf8_lossy(&cc.stderr)
        );
        let out = loft::platform::process::harness_command(&exe)
            .output()
            .expect("run the C program");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert_eq!(
        run(false),
        "true true\nnull 1000000 3 null -5\n",
        "the checked C"
    );
    assert_eq!(
        run(true),
        "true true\n-9223372036709301616 1000000 3 null -5\n",
        "the probed C"
    );
}

/// The C case mapping takes an ASCII run by its own rule (A-Z and a-z, no table search) and
/// every other character through the table, the final-sigma rule included — so a text that
/// crosses between the two answers what Rust's `str::to_lowercase` / `to_uppercase` answer.
/// Values hand-computed: in "ΣΑΣ" the first Σ precedes a letter (σ) and the last ends the
/// word (ς); ß upper-cases to "SS".
#[test]
fn the_c_case_mapping_crosses_ascii_and_the_table_as_rust_does() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let dir = scratch("case");
    let file = dir.join("case.loft");
    fa::write(
        &file,
        "fn main() {\n\
           println(\"{\\\"HeLLo ΣΑΣ wOrLd\\\".to_lowercase()}\");\n\
           println(\"{\\\"àbç straße\\\".to_uppercase()}\");\n\
           println(\"[{\\\"\\\".to_lowercase()}]\");\n\
         }\n",
    )
    .expect("write program");
    let c = dir.join("case.c");
    let emitted = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--emit-c")
        .arg(&c)
        .arg(&file)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emitted.status.success(),
        "{}",
        String::from_utf8_lossy(&emitted.stderr)
    );
    let exe = dir.join("case");
    let cc = loft::platform::process::harness_command("cc")
        .args(["-std=c99", "-O2"])
        .arg(&c)
        .arg("-o")
        .arg(&exe)
        .arg("-lm")
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .output()
        .expect("run the C program");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "hello σας world\nÀBÇ STRASSE\n[]\n"
    );
}

/// A package's `[c] shim` as C (@PLN200 B5): the shim's sources are compiled into the unit, a
/// scalar and a text binding answer what the interpreter's `dlopen`ed shim answers, a vector
/// crosses by element pointer and count, and a vector the C writes through comes back written.
/// Each value is computed by hand: `lcs_shim_sum` weights element i by i+1; `lcs_shim_fill`
/// writes (start + i) / 1000.
#[test]
fn a_c_shim_compiled_into_the_unit_answers_what_the_interpreter_answers() {
    if !have_cc() {
        eprintln!("skipped: no `cc` on PATH");
        return;
    }
    let src = "use lcshim::*;\n\
fn main() {\n\
  v = [10, 20, 30];\n\
  out: vector<float> = [0.0, 0.0, 0.0];\n\
  shim_fill(out, 1500);\n\
  px = [Px { r: 1, g: 2, b: 3 }, Px { r: 0, g: 0, b: 255 }];\n\
  println(\"{shim_mod(17, 5)} {wrapped_owned(7)} {shim_sum(v)} {out} {shim_rgb_sum(px)} {shim_half(9)}\");\n\
}\n";
    // 17 % 5 = 2; shim-7 bracketed; 10*1 + 20*2 + 30*3 = 140; 1.5, 1.501, 1.502; the records
    // as bytes: 65536 + 512 + 3 + 255 = 66306; 9 / 2 from the shim's second source.
    let want = "2 <shim-7> 140 [1.5,1.501,1.502] 66306 4\n";
    let libdir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi/pkg");
    let dir = scratch("shim");
    let file = dir.join("shim.loft");
    fa::write(&file, src).expect("write program");
    let loft = env!("CARGO_BIN_EXE_loft");
    let interp = loft::platform::process::harness_command(loft)
        .arg("--interpret")
        .arg("--lib")
        .arg(&libdir)
        .arg(&file)
        .output()
        .expect("run loft");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout),
        want,
        "the interpreter: {}",
        String::from_utf8_lossy(&interp.stderr)
    );
    let c = dir.join("shim.c");
    let emit = loft::platform::process::harness_command(loft)
        .arg("--emit-c")
        .arg(&c)
        .arg("--lib")
        .arg(&libdir)
        .arg(&file)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "the shim program is carried: {}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let unit = fa::read_to_string(&c).expect("read the C");
    // Two sources: the shim is a static archive the unit names, never two files in one
    // translation unit (each carries a `static lcs_divmod`).
    let links: Vec<&str> = unit
        .lines()
        .filter_map(|l| {
            l.strip_prefix("/* link: ")
                .and_then(|l| l.strip_suffix(" */"))
        })
        .collect();
    assert!(
        links.iter().any(|l| l.contains("lcshim_shim_")) && !unit.contains("lc_shim.c\""),
        "the two-source shim is linked as its archive, not included: {links:?}"
    );
    let exe = dir.join("shim");
    let cc = loft::platform::process::harness_command("cc")
        .args(["-std=c99", "-O1", "-DLOFT_STRICT", "-pedantic", "-Werror"])
        .arg(&c)
        .arg("-o")
        .arg(&exe)
        .arg("-lm")
        .args(&links)
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "cc rejected the shim C:\n{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .output()
        .expect("run the shim C program");
    assert_eq!(String::from_utf8_lossy(&out.stdout), want, "the C program");
}

/// @PLN203 C1 — `loft --native-c` drives the C: it emits the program, builds it with the host's
/// C compiler, caches the binary beside the program (`.loft/cache/native-c-<key>`) and runs it
/// with the program's arguments and its exit status.  The run bounds reach the C binary as they
/// reach the interpreter: `LOFT_MEMORY_LIMIT` stops a growth that crosses it and says so,
/// `LOFT_TIMEOUT` stops the run with 124.  Every value is computed by hand.
#[test]
fn native_c_builds_caches_and_runs_the_program_under_the_run_bounds() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("driver");
    let _ = fa::remove_dir_all(dir.join(".loft"));
    let file = dir.join("drive.loft");
    // 1² + … + 10² = 385; two arguments; the exit status is the argument count + 4 = 6.
    fa::write(
        &file,
        "fn main() {\n  s = 0;\n  for i in 1..=10 { s += i * i; }\n  a = arguments();\n  \
         println(\"sum {s} args {len(a)} {a[0]}{a[1]}\");\n  exit(len(a) + 4);\n}\n",
    )
    .expect("write program");
    let run = || {
        loft::platform::process::harness_command(loft)
            .arg("--native-c")
            .arg(&file)
            .args(["x", "y"])
            .env_remove("LOFT_NATIVE_NO_CACHE")
            .output()
            .expect("run loft --native-c")
    };
    for round in ["cold", "cached"] {
        let out = run();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "sum 385 args 2 xy\n",
            "{round} run: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.status.code(), Some(6), "{round} run's exit status");
    }
    let cached: Vec<String> = fa::read_dir(dir.join(".loft").join("cache"))
        .expect("the cache directory")
        .into_iter()
        .filter_map(|p| fa::file_name(&p))
        .filter(|n| n.starts_with("native-c-"))
        .collect();
    assert_eq!(
        cached.len(),
        1,
        "one cached binary for one program: {cached:?}"
    );

    // A vector of 2²⁰ integers needs 8 MiB of store; the limit is 1 MiB.
    let grow = dir.join("grow.loft");
    fa::write(
        &grow,
        "fn main() {\n  v: vector<integer> = [];\n  for i in 0..1048576 { v += [i]; }\n  \
         println(\"{len(v)}\");\n}\n",
    )
    .expect("write program");
    let out = loft::platform::process::harness_command(loft)
        .arg("--native-c")
        .arg(&grow)
        .env("LOFT_MEMORY_LIMIT", "1M")
        .output()
        .expect("run loft --native-c");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && out.stdout.is_empty(),
        "the growth past 1 MiB stops the run before it prints: {err}"
    );
    assert!(
        err.contains("store memory limit reached") && err.contains("limit 1.0 MiB"),
        "the report names the limit: {err}"
    );
    // Without the limit the same program runs to its end.
    let out = loft::platform::process::harness_command(loft)
        .arg("--native-c")
        .arg(&grow)
        .env_remove("LOFT_MEMORY_LIMIT")
        .output()
        .expect("run loft --native-c");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "1048576\n");

    // A loop no compiler can fold (the generator never reaches -1) stopped after one second.
    let spin = dir.join("spin.loft");
    fa::write(
        &spin,
        "fn main() {\n  x = 1;\n  n = 0;\n  while x != -1 { x = (x * 1103515245 + 12345) % 2147483648; \
         n += 1; }\n  println(\"{n}\");\n}\n",
    )
    .expect("write program");
    // Built first, so the timed run spends its second in the program and not in `cc`; the
    // driver's own deadline names a phase, the C's does not.
    let check = loft::platform::process::harness_command(loft)
        .args(["--check", "--native-c"])
        .arg(&spin)
        .output()
        .expect("run loft --check --native-c");
    assert_eq!(String::from_utf8_lossy(&check.stdout), "ok\n");
    let started = std::time::Instant::now();
    let out = loft::platform::process::harness_command(loft)
        .arg("--native-c")
        .arg(&spin)
        .env("LOFT_TIMEOUT", "1")
        .output()
        .expect("run loft --native-c");
    assert_eq!(
        out.status.code(),
        Some(124),
        "the deadline's status: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("[timeout] deadline reached after 1s (graceful)\n"),
        "the deadline says so: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "the bound held"
    );
}

/// @PLN203 C1 — the C logs what the interpreter logs (`src/c/loft_log.h`, a port of
/// `logger.rs`): a `log.conf` beside the program names the file, the level and a rate limit of
/// two per site, so of four `log_info` records at one line two are written; `production = true`
/// makes the failed assert and the panic records the program continues past, and the run exits
/// 1.  The lines, timestamps aside, are computed by hand and both backends must write them.
#[test]
fn the_c_logs_what_the_interpreter_logs() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let loft = env!("CARGO_BIN_EXE_loft");
    let want_log = "INFO   p.loft:2  tick 0\nINFO   p.loft:2  tick 1\nWARN   p.loft:3  warned\n\
                    ERROR  p.loft:4  [assertion_failed] assertion failed: one is two\n\
                    FATAL  p.loft:5  [user_panic] panic: gone\n";
    for backend in ["--interpret", "--native-c"] {
        let dir = scratch(&format!("log{}", backend.trim_start_matches('-')));
        let _ = fa::remove_file(dir.join("out.log"));
        fa::write(
            dir.join("log.conf"),
            "[log]\nfile = out.log\nlevel = info\nproduction = true\n\n[rate_limit]\nper_site = 2\n",
        )
        .expect("write log.conf");
        let file = dir.join("p.loft");
        fa::write(
            &file,
            "fn main() {\n  for i in 0..4 { log_info(\"tick {i}\"); }\n  log_warn(\"warned\");\n  \
             assert(1 == 2, \"one is two\");\n  panic(\"gone\");\n  println(\"after\");\n}\n",
        )
        .expect("write program");
        let out = loft::platform::process::harness_command(loft)
            .arg(backend)
            .arg(&file)
            .output()
            .expect("run loft");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "after\n",
            "{backend}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            out.status.code(),
            Some(1),
            "{backend}: a production fault counts"
        );
        let log = fa::read_to_string(dir.join("out.log")).expect("the log file");
        // `2026-10-26T14:31:51.962Z ` — the timestamp is 24 characters and a space.
        let lines: String = log
            .lines()
            .map(|l| format!("{}\n", l.get(25..).unwrap_or("")))
            .collect();
        assert_eq!(lines, want_log, "{backend}'s log");
        assert!(
            log.lines()
                .all(|l| l.len() > 24 && l.as_bytes()[23] == b'Z'),
            "{backend}: every record starts with a UTC timestamp: {log}"
        );
    }
}

/// @PLN203 C6 — `--native-c --native-wasm` writes the program as a wasm32-wasi module through
/// the C and clang, and a WASI host runs it to the interpreter's answer.  The values are
/// computed by hand: 1² + … + 10² = 385; a text built in a loop; a hash lookup.  Skipped where
/// clang, `wasm-ld` or `wasmtime` is missing.
#[test]
fn native_c_writes_a_wasm_module_a_wasi_host_runs() {
    let have = |tool: &str| {
        loft::platform::process::harness_command(tool)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if !have("clang") || !have("wasm-ld") || !have("wasmtime") {
        eprintln!("skipped: needs clang, wasm-ld and wasmtime");
        return;
    }
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("wasm");
    let file = dir.join("w.loft");
    fa::write(
        &file,
        "struct P { k: integer, v: text }\nfn main() {\n  s = 0;\n  for i in 1..=10 { s += i * i; }\n  \
         t = \"\";\n  for i in 0..4 { t += \"{i}-\"; }\n  h: hash<P[k]> = [P { k: 7, v: \"seven\" }];\n  \
         println(\"{s} {t} {h[7].v}\");\n}\n",
    )
    .expect("write program");
    let wasm = dir.join("w.wasm");
    let build = loft::platform::process::harness_command(loft)
        .args(["--native-c", "--native-wasm"])
        .arg(&wasm)
        .arg(&file)
        .output()
        .expect("run loft --native-wasm");
    if String::from_utf8_lossy(&build.stderr).contains("toolchain is incomplete") {
        eprintln!("skipped: no WASI sysroot for clang");
        return;
    }
    assert!(
        build.status.success(),
        "the module builds: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = loft::platform::process::harness_command("wasmtime")
        .args(["-W", "exceptions=y"])
        .arg(&wasm)
        .output()
        .expect("run wasmtime");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "385 0-1-2-3- seven\n",
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// @PLN203 C5 — `loft --tests --native-c` runs a file's tests on the C: one unit, one process
/// per test, each test's verdict reported as the interpreter's runner reports it.  `sq_bad`
/// asserts 3² = 10 and fails with its message; the other two pass; a test's stores do not
/// reach the next (`counts` sees an empty vector though `fills` ran before it).
#[test]
fn native_c_runs_a_files_tests_one_process_each() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("ctests");
    let file = dir.join("t.loft");
    fa::write(
        &file,
        "fn sq(x: integer) -> integer { x * x }\nfn fills() { v: vector<integer> = [1, 2, 3]; \
         assert(len(v) == 3, \"three\"); }\nfn sq_bad() { assert(sq(3) == 10, \"ten\"); }\n\
         fn counts() { w: vector<integer> = []; assert(len(w) == 0, \"empty\"); }\n",
    )
    .expect("write tests");
    for backend in ["--native-c", "--interpret"] {
        // Under the leak gate, as the language suite runs `loft test`: a store the C leaves
        // behind would fail a test that passes.
        let out = loft::platform::process::harness_command(loft)
            .args(["--tests", backend])
            .arg(&file)
            .env("LOFT_LEAK_CHECK", "1")
            .output()
            .expect("run loft --tests");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            out.status.code(),
            Some(1),
            "{backend}: one test fails\n{all}"
        );
        assert!(
            all.contains("t.loft::sq_bad  —  assertion failed: ten"),
            "{backend}: the failure names its test and message\n{all}"
        );
        assert!(
            all.contains("1 failed; 2 passed; 3 total"),
            "{backend}: the other two pass\n{all}"
        );
    }
}

/// @PLN203 C4 — `par` on threads in the C (`loft_par_run`): a scalar worker, one with a captured
/// extra, and a text worker over records each run on four threads and answer what the rows give
/// in order; `LOFT_TRACE_PAR_WORKERS` reports four workers for each and ONE for `par(…, 1)`, so
/// the instrument can fail.  Hand-computed: Σi² over 0..10000 = 9999·10000·19999/6 =
/// 333283335000; 3·Σi = 3·49995000 = 149985000; the labels `n0`…`n99` are 10·2 + 90·3 = 290
/// characters.
#[test]
fn par_runs_on_threads_in_the_c_and_answers_as_in_order() {
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
  v: vector<integer> = [];
  for i in 0..10000 { v += [i]; }
  s = 0;
  for x in v par(r = sq(x), 4) { s += r; }
  k = 3;
  u = 0;
  for x in v par(m = scaled(x, k), 4) { u += m; }
  items: vector<It> = [];
  for i in 0..100 { items += [It { name: "n", n: i }]; }
  t = 0;
  for it in items par(l = label(it), 4) { t += len(l); }
  one = 0;
  for x in v par(r = sq(x), 1) { one += r; }
  println("{s} {u} {t} {one}");
}
"#;
    let want = "333283335000 149985000 290 333283335000\n";
    let dir = scratch("parthreads");
    let file = dir.join("p.loft");
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
    let out = loft::platform::process::harness_command(loft)
        .arg("--native-c")
        .arg(&file)
        .env("LOFT_TRACE_PAR_WORKERS", "1")
        .output()
        .expect("run loft --native-c");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        want,
        "the C: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let workers: Vec<String> = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter_map(|l| l.split("distinct_workers=").nth(1).map(str::to_string))
        .collect();
    assert_eq!(workers, ["4", "4", "4", "1"], "workers per par loop");
}

/// @PLN203 C6 — `--native-c --html` writes a page that hosts the program's wasm32-wasi module
/// with a WASI layer of its own: the page's script, run here under Node with a `document` of two
/// methods, writes what the program prints into the page and records the exit status.
/// Hand-computed: Σi² over 1..=10 = 385; a text built in a loop; a hash lookup; `exit(3)`.
/// Skipped where clang, `wasm-ld` or Node is missing.
#[test]
fn native_c_html_page_runs_its_program() {
    let have = |tool: &str| {
        loft::platform::process::harness_command(tool)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if !have("clang") || !have("wasm-ld") || !have("node") {
        eprintln!("skipped: needs clang, wasm-ld and node");
        return;
    }
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("html");
    let file = dir.join("page.loft");
    fa::write(
        &file,
        "struct P { k: integer, v: text }\nfn main() {\n  s = 0;\n  for i in 1..=10 { s += i * i; }\n  \
         t = \"\";\n  for i in 0..4 { t += \"{i}-\"; }\n  h: hash<P[k]> = [P { k: 7, v: \"seven\" }];\n  \
         println(\"{s} {t} {h[7].v}\");\n  exit(3);\n}\n",
    )
    .expect("write program");
    let html = dir.join("page.html");
    let build = loft::platform::process::harness_command(loft)
        .args(["--native-c", "--html"])
        .arg(&html)
        .arg(&file)
        .output()
        .expect("run loft --html");
    if String::from_utf8_lossy(&build.stderr).contains("toolchain is incomplete") {
        eprintln!("skipped: no WASI sysroot for clang");
        return;
    }
    assert!(
        build.status.success(),
        "the page builds: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let runner = dir.join("run.js");
    fa::write(
        &runner,
        "const fs = require('fs');\n\
         const html = fs.readFileSync(process.argv[2], 'utf8');\n\
         const js = html.split('<script id=\"loft-run\">')[1].split('</script>')[0];\n\
         let text = '';\n\
         globalThis.window = globalThis;\n\
         globalThis.document = { title: '',\n\
           getElementById: () => ({ appendChild: (n) => { text += n.data !== undefined ? n.data : n.textContent; } }),\n\
           createTextNode: (t) => ({ data: t }),\n\
           createElement: () => ({ className: '', textContent: '' }) };\n\
         eval(js);\n\
         const t = setInterval(() => { if (window.loftExit !== undefined) { clearInterval(t);\n\
           process.stdout.write(text + 'exit=' + window.loftExit + '\\n'); } }, 10);\n",
    )
    .expect("write runner");
    let out = loft::platform::process::harness_command("node")
        .arg(&runner)
        .arg(&html)
        .output()
        .expect("run node");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "385 0-1-2-3- seven\nexit=3\n",
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Does `cc` build and run an AddressSanitizer program with its leak check on?  Probed under
/// the options the test runs with, so a host whose sanitizer has no leak check (macOS's) or a
/// box without the runtime skips rather than fails.
fn have_asan(dir: &std::path::Path) -> bool {
    if !have_cc() {
        return false;
    }
    let src = dir.join("asan_probe.c");
    let exe = dir.join("asan_probe");
    fa::write(&src, "int main(void) { return 0; }\n").expect("write probe");
    loft::platform::process::harness_command("cc")
        .args(["-fsanitize=address"])
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .is_ok_and(|o| o.status.success())
        && loft::platform::process::harness_command(&exe)
            .env("ASAN_OPTIONS", "detect_leaks=1")
            .output()
            .is_ok_and(|o| o.status.success())
}

/// `prog` written as C by the `loft` binary, built under AddressSanitizer and UBSan, run from
/// `cwd` with the leak check on: its exit status, stdout and stderr.
fn run_asan(
    dir: &std::path::Path,
    prog: &std::path::Path,
    cwd: &std::path::Path,
) -> (i32, String, String) {
    let stem = fa::file_stem(prog).unwrap_or_else(|| "p".into());
    let c = dir.join(format!("{stem}.c"));
    let exe = dir.join(&stem);
    let emit = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--emit-c")
        .arg(&c)
        .arg(prog)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "{stem}: {}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let inc = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/c");
    let cc = loft::platform::process::harness_command("cc")
        .args(["-std=c99", "-O1", "-g", "-DLOFT_CALL_CHAIN", "-pthread"])
        .args(["-fsanitize=address,undefined", "-fno-omit-frame-pointer"])
        .arg(format!("-I{}", inc.display()))
        .arg(&c)
        .args(unit_links(&c))
        .arg("-o")
        .arg(&exe)
        .args(["-lm", "-ldl"])
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "{stem}: {}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .current_dir(cwd)
        .env("ASAN_OPTIONS", "detect_leaks=1")
        .env_remove("LOFT_SOURCE_DIR")
        .output()
        .expect("run the program");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Faults that end a test inside frames holding texts, for the leak check below: a 10 000-deep
/// recursion (the hold stack grows), a tuple's texts, and a lazy driver's CONTAINED fault, twenty
/// times, under a caller whose own text must survive each one.  Beside them a passing test whose
/// generic loop moves a callback's `(integer, text)` into one local per pass: on the build before
/// the fix every pass but the last left the old tuple's text allocated.
const HALTS: &str = r#"
fn down(n: integer) -> text {
  own = "level {n}";
  if n == 0 {
    assert(false, "bottom {own}");
  }
  r = down(n - 1);
  "{own}/{r}"
}

// @EXPECT_FAIL: bottom level 0
fn test_a_deep_recursion_halts() {
  x = down(10000);
  assert(len(x) > 0, "never");
}

fn pair(k: integer) -> (text, text) {
  ("left {k}", "right {k}")
}

// @EXPECT_FAIL: tuple left 7 right 7
fn test_a_tuple_of_texts_at_a_halt() {
  p = pair(7);
  (a, b) = p;
  assert(len(a) < 0, "tuple {a} {b}");
}

struct LuPerson { id: integer, nm: text }
struct LuWorld { people: hash<LuPerson[id]> = [] }

fn lazy_fetch(coll: hash<LuPerson[id]>, source: text, key_int: integer, key_text: text) -> integer {
  mine = "driver {key_int} {source}";
  assert(key_int != 1, "driver faults holding {mine}");
  coll += [LuPerson { id: key_int, nm: "ok-{key_int}" }];
  1
}

fn test_a_contained_fault_keeps_the_callers_texts() {
  outer = "caller {6 * 7}";
  w = LuWorld { };
  _ = store_bind_lazy(w.people, "duckdb:unwind");
  for _i in 0..20 {
    r = w.people[1];
    assert(r == null, "a faulted fetch answers null");
  }
  after = "{outer} still {len(outer)}";
  assert(after == "caller 42 still 9", "got {after}");
  two = w.people[2];
  assert(two != null && two.nm == "ok-2", "the next fetch inserts");
}

fn test_after_them() {
  s = "after {10 + 1}";
  assert(s == "after 11", "got {s}");
}

fn app<U>(v: vector<integer>, f: fn(integer) -> U) -> vector<U> {
  out: vector<U> = [];
  for c in v { out += [f(c)]; }
  out
}

fn test_a_generic_callback_answers_tuples_of_texts() {
  t = app([1, 2, 3], |x| { (x, "w{x}") });
  assert(len(t) == 3 && t[0].1 == "w1" && t[2].1 == "w3" && t[1].0 == 2, "{t[1].1}");
}
"#;

/// loft#2198: a failed assert or a panic ends a C test by `longjmp`, past every scope-exit free,
/// so each text its frames held stayed allocated; a lazy driver's contained fault did the same
/// once per failed fetch.  Each function now names the texts its epilogue frees and the halt
/// releases them before it jumps — a contained one only above the driver.  Scored by
/// LeakSanitizer: on the build before the fix, 2121's six functions leaked 35 allocations, the
/// recursion and the tuple 58 leak records, and the driver one text per contained fault.  A
/// release that reached a live caller's text would show here as a use-after-free, or as
/// `caller 42` read wrong.
///
/// loft#2199: the program-directory anchor was a `loft_text` used as a C string with no
/// terminator, so `chdir` failed on the bytes after it (and the fault log read past it): run
/// from `/`, `directory()` answered `/` instead of the program's own directory.
#[test]
fn a_halted_c_test_releases_the_texts_its_frames_hold() {
    let dir = scratch("halt-release");
    if !have_asan(&dir) {
        eprintln!("skipped: no AddressSanitizer runtime for cc here");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let halts = dir.join("halts.loft");
    fa::write(&halts, HALTS).expect("write halts.loft");
    let guard = root.join("tests/scripts/2121-a-halted-frame-releases-the-texts-it-holds.loft");
    for prog in [guard.as_path(), halts.as_path()] {
        let (code, _, err) = run_asan(&dir, prog, &dir);
        assert!(
            code == 0 && !err.contains("Sanitizer") && !err.contains("runtime error:"),
            "{}: exit {code}\n{err}",
            prog.display()
        );
    }
    let anchor = dir.join("anchor.loft");
    fa::write(
        &anchor,
        "fn main() {\n  println(\"dir {directory()}\");\n}\n",
    )
    .expect("write");
    let (code, out, err) = run_asan(&dir, &anchor, std::path::Path::new("/"));
    let want = fa::canonical(&dir).expect("canonical scratch").native();
    assert_eq!(
        (code, out.trim_end()),
        (0, format!("dir {want}").as_str()),
        "{err}"
    );
}

/// @PLN203 — the natives of an `in_binary` library (`[native] in_binary = true`: implemented in
/// loft's own binary) run on the C when its runtime carries them, and refuse by name when it
/// does not.  `lib/process`'s `process_run` is carried (`loft_process.h`): the values below are
/// computed by hand — a run's code and both streams, a signal as 128 + n, a program that cannot
/// start, a timeout's -2, a cursor's lines — and the interpreter must give the same lines.
/// `engine_host`'s `rebuild_status` is not carried: the C must refuse the unit naming it, never
/// emit the "not loaded" stub, which the interpreter cannot give for a native of its own binary.
#[test]
fn in_binary_natives_run_on_the_c_or_refuse_by_name() {
    if !have_cc() || !loft::platform::c_runtime_posix_host() {
        eprintln!("skipped: no cc on PATH, or not a POSIX host");
        return;
    }
    let loft = env!("CARGO_BIN_EXE_loft");
    let lib = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lib");
    let dir = scratch("inbinary");
    let file = dir.join("p.loft");
    fa::write(
        &file,
        "use process::*;\nfn main() {\n  \
         both: Command = \"sh -c 'echo out; echo err >&2; exit 3'\";\n  b = both.run();\n  \
         println(\"run {b.ok} {b.code} [{b.stdout.trim()}] [{b.stderr.trim()}]\");\n  \
         killed: Command = \"sh -c 'kill -TERM $$'\";\n  println(\"signal {killed.run().code}\");\n  \
         missing: Command = \"no-such-program-for-loft\";\n  m = missing.run();\n  \
         println(\"missing {m.code} {m.stderr.starts_with(\\\"no-such-program-for-loft: \\\")}\");\n  \
         slow: Command = \"sleep 5\";\n  t = slow.run(\"\", 100);\n  \
         println(\"timeout {t.code} {t.timed_out}\");\n  \
         three: Command = \"sh -c 'echo a; echo b >&2; echo c'\";\n  n = 0;\n  \
         for l in three.lines() { n += 1; println(\"line {n} {l}\"); }\n}\n",
    )
    .expect("write program");
    let want = "run false 3 [out] [err]\nsignal 143\nmissing -1 true\ntimeout -2 true\n\
                line 1 a\nline 2 c\n";
    for backend in ["--native-c", "--interpret"] {
        let out = loft::platform::process::harness_command(loft)
            .arg(backend)
            .arg("--lib")
            .arg(&lib)
            .arg(&file)
            .env("LOFT_TIMEOUT", "120")
            .output()
            .expect("run loft");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            want,
            "{backend}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let host = dir.join("h.loft");
    fa::write(
        &host,
        "use engine_host;\nfn main() { println(\"{engine_host::rebuild_status()}\"); }\n",
    )
    .expect("write program");
    let out = loft::platform::process::harness_command(loft)
        .args(["--emit-c"])
        .arg(dir.join("h.c"))
        .arg("--lib")
        .arg(&lib)
        .arg(&host)
        .output()
        .expect("run loft --emit-c");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "a refusal: {err}");
    assert!(
        err.contains("rebuild_status") && err.contains("the C runtime does not carry it"),
        "the refusal names the native: {err}"
    );
}

/// @PLN203 C5 — `lib/engine_host`'s kernel on the C (`loft_engine.h`) and on the interpreter,
/// driven from outside by a WebSocket and a UDP peer, every message written out by hand: the
/// 101 response carries the UDP cookie (`X-Loft-UDP`); the connect handler's greeting arrives
/// (`hello 0`, the first slot's cid); a text frame is echoed; `H:<cookie>` is acked `A:0`; a
/// seq-stamped datagram of a declared sync kind reaches `on_event` at the next tick and is
/// echoed over the WebSocket; `quit` stops the run and the program exits 0.
#[test]
fn the_engine_host_kernel_serves_the_same_on_the_c_and_the_interpreter() {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};
    if !have_cc() || !loft::platform::c_runtime_posix_host() {
        eprintln!("skipped: no cc on PATH, or not a POSIX host");
        return;
    }
    let loft = env!("CARGO_BIN_EXE_loft");
    let lib = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lib");
    for backend in ["--native-c", "--interpret"] {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .expect("a free port");
        let dir = scratch(&format!("kernel{}", &backend[2..]));
        let file = dir.join("k.loft");
        fa::write(
            &file,
            format!(
                "use engine_host;\nstruct W {{ n: integer not null }}\nfn main() {{\n  \
                 w = W {{ n: 0 }};\n  engine_host::sync_class(7);\n  \
                 engine_host::run({port}, 5000,\n    fn(ev: engine_host::Event) {{\n      \
                 if ev.kind == 0 {{ engine_host::send(ev.cid, \"hello {{ev.cid}}\"); }}\n      \
                 if ev.kind == 1 {{\n        \
                 if ev.payload == \"quit\" {{ engine_host::stop(); return; }}\n        \
                 engine_host::broadcast(\"echo:{{ev.payload}}\");\n      }}\n    }},\n    \
                 fn() {{ w.n += 1; }});\n}}\n"
            ),
        )
        .expect("write program");
        let mut child = loft::platform::process::harness_command(loft)
            .arg(backend)
            .arg("--lib")
            .arg(&lib)
            .arg(&file)
            .env("LOFT_TIMEOUT", "120")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("start the kernel");
        // The C build compiles first: wait for the listener.
        let deadline = Instant::now() + Duration::from_secs(90);
        let ws = loop {
            if let Ok(s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
                break s;
            }
            if Instant::now() > deadline || child.try_wait().ok().flatten().is_some() {
                let _ = child.kill();
                let out = child.wait_with_output().expect("kernel output");
                panic!(
                    "{backend}: the kernel never listened\n{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        ws.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        (&ws)
            .write_all(
                b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
                  Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
            )
            .unwrap();
        let mut head = Vec::new();
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            (&ws).read_exact(&mut b).unwrap();
            head.push(b[0]);
        }
        let head = String::from_utf8_lossy(&head).to_string();
        assert!(head.starts_with("HTTP/1.1 101 "), "{backend}: {head}");
        assert!(
            head.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"),
            "{backend}: the RFC 6455 sample key's accept value: {head}"
        );
        let cookie = head
            .lines()
            .find_map(|l| l.strip_prefix("X-Loft-UDP: "))
            .unwrap_or_else(|| panic!("{backend}: no cookie: {head}"))
            .trim()
            .to_string();
        assert_eq!(cookie.len(), 16, "{backend}: a 16-hex cookie: {cookie}");
        let recv = || -> String {
            let mut h = [0u8; 2];
            (&ws).read_exact(&mut h).unwrap();
            assert_eq!(h[0], 0x81, "{backend}: an unmasked final text frame");
            let mut payload = vec![0u8; usize::from(h[1] & 0x7F)];
            (&ws).read_exact(&mut payload).unwrap();
            String::from_utf8(payload).unwrap()
        };
        let send = |text: &str| {
            let mask = [0x21u8, 0x43, 0x65, 0x87];
            let mut frame = vec![0x81u8, 0x80 | u8::try_from(text.len()).unwrap()];
            frame.extend_from_slice(&mask);
            frame.extend(text.bytes().enumerate().map(|(i, c)| c ^ mask[i % 4]));
            (&ws).write_all(&frame).unwrap();
        };
        assert_eq!(recv(), "hello 0", "{backend}: the connect greeting");
        send("ping");
        assert_eq!(recv(), "echo:ping", "{backend}: the echo");
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        udp.set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let mut ack = String::new();
        for _ in 0..20 {
            udp.send_to(format!("H:{cookie}").as_bytes(), ("127.0.0.1", port))
                .unwrap();
            let mut buf = [0u8; 64];
            if let Ok((n, _)) = udp.recv_from(&mut buf) {
                ack = String::from_utf8_lossy(&buf[..n]).to_string();
                break;
            }
        }
        assert_eq!(ack, "A:0", "{backend}: the hello's ack");
        udp.send_to(b"S:5:7:pose", ("127.0.0.1", port)).unwrap();
        assert_eq!(
            recv(),
            "echo:7:pose",
            "{backend}: the sync datagram, at the next tick"
        );
        send("quit");
        let out = child.wait_with_output().expect("kernel output");
        assert!(
            out.status.success(),
            "{backend}: exit {:?}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(&format!(
                "engine_host: listening on ws://0.0.0.0:{port}/ (tick every 5000 us)"
            )),
            "{backend}: the listen line"
        );
    }
}

/// `@FR-R-FileHold` as C (`lower::file_hold::held_file`, native's gate): a loop that reads one
/// `File` through fixed-width integer reads keeps the record's cursor in a `loft_fcur` — read
/// once, served from a 4 KB buffer, given back on EVERY exit: the loop's end, a `break` of an
/// outer loop from inside it, a `return` from inside it.  Loft has no exception, so those are
/// all the exits.  Beside it the deep push window (`hoist::deep_window_ok`), which the reading
/// loop of cell 1 pushes through.  The program checks its own hand-computed values — the k-th
/// little-endian i16 of the bytes 0, 1, 2, ... is 514k + 256, the big-endian one 514k + 1 —
/// and the record's position after each exit; the C must print its `ok` in the strict and the
/// verify builds.
#[test]
fn a_held_file_cursor_gives_its_position_back_on_every_exit() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"// The k-th little-endian i16 of bytes 0, 1, 2, ... is 514k + 256; big-endian 514k + 1.
fn one(f: File) -> integer { f#read(2) as i16 as integer }

fn first_over(f: File, limit: integer) -> integer {
  k = 0;
  while k < 100 {
    v = f#read(2) as i16 as integer;
    if v > limit { return k; }
    k += 1;
  }
  -1
}

fn main() {
  dir = "{temp_dir() ?? "/tmp"}/loft_fh_{ticks()}";
  assert(mkdir(dir) == FileResult.Ok, "mkdir");
  p = "{dir}/b.bin";
  bytes: vector<u8> = [];
  for i in 0..40 { bytes += [(i & 255) as u8]; }
  assert(write_bytes(p, bytes), "write");

  // 1. A whole loop, then a read through the record: positions 0..4, then 5.
  f1 = file(p);
  f1#format = LittleEndian;
  s1 = 0;
  for _k in 0..5 { s1 += f1#read(2) as i16 as integer; }
  assert(s1 == 514 * 10 + 256 * 5, "1 loop: {s1}");
  assert(one(f1) == 514 * 5 + 256, "1 after: {one(f1)}");

  // 2. A break after three: the record goes on at the fourth.
  f2 = file(p);
  f2#format = LittleEndian;
  n2 = 0;
  for _k in 0..10 { _x = f2#read(2) as i16 as integer; n2 += 1; if n2 == 3 { break; } }
  assert(one(f2) == 514 * 3 + 256, "2 after a break");

  // 3. A return from inside the loop: values 256, 770, 1284, ... the first over 1500 is k = 3
  //    (1798), and the caller reads on at the fifth.
  f3 = file(p);
  f3#format = LittleEndian;
  assert(first_over(f3, 1500) == 3, "3 return");
  assert(one(f3) == 514 * 4 + 256, "3 after a return");

  // 4. A break of the OUTER loop from the inner one, which alone holds the file (the outer
  //    loop names the file outside a read): two rounds of two reads, the third round's first
  //    read breaks out — five reads, and the record goes on at the sixth.
  f4 = file(p);
  f4#format = LittleEndian;
  rounds = 0;
  for r in 0..5 {
    rounds += 1;
    if f4#format != LittleEndian { break; }
    for k in 0..2 {
      _y = f4#read(2) as i16 as integer;
      if r == 2 && k == 0 { r#break; }
    }
  }
  assert(rounds == 3, "4 rounds: {rounds}");
  assert(one(f4) == 514 * 5 + 256, "4 after an outer break");

  // 5. Past the end: 20 values, the 21st read answers null and the cursor stays.
  f5 = file(p);
  f5#format = LittleEndian;
  got = 0;
  nulls = 0;
  for _k in 0..22 { v = f5#read(2) as i16; if v == null { nulls += 1; } else { got += 1; } }
  assert(got == 20 && nulls == 2, "5 end: {got} {nulls}");

  // 6. Big-endian, the file's default.
  f6 = file(p);
  s6 = 0;
  for _k in 0..4 { s6 += f6#read(2) as i16 as integer; }
  assert(s6 == 514 * 6 + 4, "6 big-endian: {s6}");

  // 7. Across the cursor's 4096-byte block: one byte, then 2499 i16s at odd offsets, one of
  //    them straddling byte 4096; the expected sum is taken from the byte vector itself.
  p7 = "{dir}/c.bin";
  big: vector<u8> = [];
  for i in 0..5000 { big += [((i * 7) & 255) as u8]; }
  assert(write_bytes(p7, big), "write 7");
  want = 0;
  for k in 0..2499 {
    lo = (big[1 + 2 * k] ?? 0) as integer;
    hi = (big[2 + 2 * k] ?? 0) as integer;
    v = lo + hi * 256;
    if v >= 32768 { v = v - 65536; }
    want += v;
  }
  f7 = file(p7);
  f7#format = LittleEndian;
  first = f7#read(1) as u8 as integer;
  s7 = 0;
  for _k in 0..2499 { s7 += f7#read(2) as i16 as integer; }
  assert(first == 0 && s7 == want, "7 across the block: {first} {s7} {want}");
  assert(delete(p7) == FileResult.Ok, "cleanup 7");

  assert(delete(p) == FileResult.Ok && rmdir(dir) == FileResult.Ok, "cleanup");
  println("ok");
}
"#;
    let loft = env!("CARGO_BIN_EXE_loft");
    let dir = scratch("file-hold");
    let tmp = dir.join("tmp");
    fa::create_dir_all(&tmp).expect("make the temporary directory");
    let file = dir.join("hold.loft");
    fa::write(&file, src).expect("write program");
    let interp = loft::platform::process::harness_command(loft)
        .arg("--interpret")
        .arg(&file)
        .env("TMPDIR", &tmp)
        .output()
        .expect("run loft");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout),
        "ok\n",
        "the interpreter"
    );
    let c = dir.join("hold.c");
    let emit = loft::platform::process::harness_command(loft)
        .arg("--emit-c")
        .arg(&c)
        .arg(&file)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "{}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let text = fa::read_to_string(&c).expect("read the C");
    let holds = text.matches("loft_fcur_hold(").count();
    let releases = text.matches("loft_fcur_release(").count();
    assert!(holds >= 6, "the reading loops hold their file: {holds}");
    assert!(
        releases > holds,
        "the return and the outer break give the cursor back too: {releases} for {holds}"
    );
    for (mode, flags) in [
        ("strict", &["-DLOFT_STRICT", "-pedantic", "-Werror"][..]),
        ("verify", &["-DLOFT_VERIFY", "-w"][..]),
    ] {
        let exe = dir.join(mode);
        let cc = loft::platform::process::harness_command("cc")
            .args(["-std=c99", "-O1"])
            .args(flags)
            .arg(&c)
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .output()
            .expect("run cc");
        assert!(
            cc.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&cc.stderr)
        );
        let run = loft::platform::process::harness_command(&exe)
            .env("TMPDIR", &tmp)
            .output()
            .expect("run the C");
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "ok\n",
            "{mode}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

/// @PLN203 C4 tier 2 — `par` whose worker answers a RECORD runs on threads in the C: each worker
/// makes its records in stores of its own table, and after it joined the caller copies each row
/// it computed into one result store — the record's bytes and the claims it owns, as
/// `Stores::copy_from_worker` does — before the worker's stores go (`loft_par_adopt`).  The rows:
/// a struct of scalars, one that owns a text and a vector (the claims), a struct-enum, a
/// pass-through of the input element (a store the caller had, not copied), and `par(…, 1)`.
/// Four workers ran each threaded loop, one the last, so the instrument can fail.  Hand-computed:
/// Σ(a + b) = 3·Σx = 3·499500 = 1498500; the names `q0`…`q999` are 10·2 + 90·3 + 900·4 = 3890
/// characters; each `xs` sums to 3x + 3, so 1498500 + 3000 = 1501500; Σk = 1498500; the even
/// rows' Dots sum to 249500, the odd rows' Bars are 500 of width 10.
#[test]
fn par_returns_records_from_threads_in_the_c() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"struct P { a: integer, b: integer }
struct Q { name: text, xs: vector<integer>, k: integer }
enum Shape { Dot { at: integer }, Bar { lo: integer, hi: integer } }
struct It { n: integer }
fn pair(x: integer) -> P { P { a: x, b: x * 2 } }
fn owned(x: integer) -> Q { Q { name: "q{x}", xs: [x, x + 1, x + 2], k: x * 3 } }
fn shape(x: integer) -> Shape { if x % 2 == 0 { Dot { at: x } } else { Bar { lo: x, hi: x + 10 } } }
fn same(it: const It) -> It { it }
fn main() {
  v: vector<integer> = [];
  for i in 0..1000 { v += [i]; }
  // 1. scalars only: sum of a + b = 3 * sum(0..1000) = 3 * 499500 = 1498500
  s1 = 0;
  for x in v par(r = pair(x), 4) { s1 += r.a + r.b; }
  // 2. owned text and vector: names q0..q999 are 10*2 + 90*3 + 900*4 = 3890 characters;
  //    each xs sums to 3x + 3 -> 3 * 499500 + 3000 = 1501500; k sums to 1498500.
  c2 = 0; x2 = 0; k2 = 0;
  for x in v par(q = owned(x), 4) { c2 += len(q.name); for e in q.xs { x2 += e; } k2 += q.k; }
  // 3. a struct-enum: even x a Dot at x (sum 249500), odd a Bar of width 10 (500 * 10)
  d3 = 0; w3 = 0;
  for x in v par(sh = shape(x), 4) {
    match sh { Dot { at } => { d3 += at; }, Bar { lo, hi } => { w3 += hi - lo; } }
  }
  // 4. a pass-through of the input element: sum of n = 499500
  items: vector<It> = [];
  for i in 0..1000 { items += [It { n: i }]; }
  s4 = 0;
  for it in items par(r = same(it), 4) { s4 += r.n; }
  // 5. one thread
  s5 = 0;
  for x in v par(r = pair(x), 1) { s5 += r.a + r.b; }
  println("{s1} {c2} {x2} {k2} {d3} {w3} {s4} {s5}");
}
"#;
    let want = "1498500 3890 1501500 1498500 249500 5000 499500 1498500\n";
    let dir = scratch("parrecords");
    let file = dir.join("p.loft");
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
    let out = loft::platform::process::harness_command(loft)
        .arg("--native-c")
        .arg(&file)
        .env("LOFT_TRACE_PAR_WORKERS", "1")
        .output()
        .expect("run loft --native-c");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        want,
        "the C: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let workers: Vec<String> = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter_map(|l| l.split("distinct_workers=").nth(1).map(str::to_string))
        .collect();
    assert_eq!(workers, ["4", "4", "4", "4", "1"], "workers per par loop");
}

/// The C a program's constants are written into is the same on every run, and the same when
/// the backend reads the program image instead of the program in memory (@PLN205 L0b).  A
/// constant's store is copied into the unit word for word, so a stale byte from a reused store
/// made two runs differ (heap addresses move between runs), and the image carried no built
/// constants at all until they were rebuilt from its IR.
#[test]
fn a_programs_constants_are_written_the_same_every_run_and_from_the_image() {
    let dir = scratch("consts");
    let file = dir.join("consts.loft");
    fa::write(
        &file,
        "struct Row { id: integer, name: text, on: boolean }\n\
         const ROWS: vector<Row> = [Row { id: 1, name: \"one\", on: true }, Row { id: 2, name: \"two\", on: false }];\n\
         const PRIMES: vector<integer> = [2, 3, 5, 7, 11];\n\
         fn main() {\n  println(\"{ROWS[1].name} {PRIMES[4]} {ROWS.len()}\");\n}\n",
    )
    .expect("write program");
    let emit = |out: &str, image: bool| -> Vec<u8> {
        let target = dir.join(out);
        let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
        cmd.arg("--emit-c").arg(&target).arg(&file);
        if image {
            cmd.env("LOFT_EMIT_FROM_IMAGE", "1");
        } else {
            cmd.env_remove("LOFT_EMIT_FROM_IMAGE");
        }
        let o = cmd.output().expect("run loft");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        fa::read(&target).expect("the unit")
    };
    let first = emit("a.c", false);
    assert!(
        String::from_utf8_lossy(&first).contains("loft_cwords"),
        "the unit carries the constants' words"
    );
    assert!(first == emit("b.c", false), "two runs write the same C");
    assert!(
        first == emit("c.c", true),
        "the image writes the same C as memory"
    );
}

/// Run `src` as `name` on the interpreter and as C (strict C99, `-Werror`), each answering
/// `want`; answers the C the emitter wrote.
fn both_answer(name: &str, src: &str, want: &str) -> String {
    let dir = scratch(name);
    let file = dir.join(format!("{name}.loft"));
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
        "the interpreter: {}",
        String::from_utf8_lossy(&interp.stderr)
    );
    let c = dir.join(format!("{name}.c"));
    let emit = loft::platform::process::harness_command(loft)
        .arg("--emit-c")
        .arg(&c)
        .arg(&file)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "{name} is carried: {}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let exe = dir.join(name);
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
        "cc rejected {name}:\n{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = loft::platform::process::harness_command(&exe)
        .output()
        .expect("run the C program");
    assert_eq!(String::from_utf8_lossy(&out.stdout), want, "the C program");
    fa::read_to_string(&c).expect("read the C")
}

/// A small integer in a `vector<i32>` literal is a value, not a function: the reachability walk
/// read `20` written into a 4-byte element as the definition number of an interface stub, and
/// the C backend refused a stub it cannot write (`iface_18#2_compare`).  The three values are
/// the ones that named stubs; the program prints them back and their sum.
#[test]
fn a_small_integer_in_a_narrow_vector_is_not_a_function() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"
fn main() {
  v: vector<i32> = [30, 10, 20];
  t = 0;
  for e in v { t += e; }
  println("{v} {t}");
}
"#;
    both_answer("narrow_vec", src, "[30,10,20] 60\n");
}

/// @PLN204 — a `#width 32` file's arithmetic is 32-bit on the C backend too: the loop's locals
/// are `int32_t` and its sum is `loft_add32`, and a sum that leaves the `i32` range reads null
/// as the interpreter's `OpNarrow32` answers it.  0+1+2+3 = 6; 0+..+69999 = 2449965000 > i32::MAX.
#[test]
fn a_width32_loop_is_written_in_int32() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let src = r#"#width 32
fn sum_to(n: integer) -> integer {
  t = 0;
  for i in 0..n { t += i; }
  t
}
fn main() {
  println("{sum_to(4)} {sum_to(70000)}");
}
"#;
    let c = both_answer("width32_loop", src, "6 null\n");
    // The body that holds the loop (the frame-keeping wrapper, where there is one, only calls it).
    let body = c
        .split("static int32_t n_sum_to")
        .skip(1)
        .find(|b| b.contains("for (;;)"))
        .and_then(|b| b.split("\n}\n").next())
        .unwrap_or_else(|| panic!("sum_to returning int32_t, with its loop, in:\n{c}"));
    assert!(body.contains("int32_t v0_n"), "a 32-bit parameter:\n{body}");
    assert!(body.contains("loft_add32(v1_t, "), "a 32-bit sum:\n{body}");
    assert!(!body.contains("int64_t"), "no 64-bit local:\n{body}");
}

/// A C build never builds a library's Rust native crate: the C links `[c] shim`s and loft's
/// components only, and the cargo build of a crate it would never load cost a `use graphics`
/// over 25 minutes before a word of C (`native_build::c_target`).  The library's crate here
/// cannot compile; without the rule `--emit-c` still tries, leaving `native-auto/` behind.
#[test]
fn a_c_build_builds_no_rust_native_crate() {
    let dir = scratch("c_target_no_crate");
    let lib = dir.join("libs/rustlib");
    fa::create_dir_all(lib.join("src")).expect("lib dir");
    fa::create_dir_all(lib.join("native/src")).expect("crate dir");
    fa::write(
        lib.join("loft.toml"),
        "[package]\nname = \"rustlib\"\nversion = \"0.1.0\"\nloft = \">=2026.10.0\"\n\n\
         [library]\nentry = \"src/rustlib.loft\"\n\n[native]\ncrate = \"rustlib-native\"\n",
    )
    .expect("manifest");
    fa::write(
        lib.join("src/rustlib.loft"),
        "pub fn twice(x: integer) -> integer { x * 2 }\n",
    )
    .expect("library");
    fa::write(
        lib.join("native/Cargo.toml"),
        "[package]\nname = \"rustlib-native\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
         [lib]\ncrate-type = [\"cdylib\"]\n",
    )
    .expect("crate manifest");
    fa::write(lib.join("native/src/lib.rs"), "this is not rust\n").expect("crate source");
    let prog = dir.join("p.loft");
    fa::write(
        &prog,
        "use rustlib;\nfn main() { println(\"{rustlib::twice(21)}\"); }\n",
    )
    .expect("program");
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--emit-c")
        .arg(dir.join("p.c"))
        .arg("--lib")
        .arg(dir.join("libs"))
        .arg(&prog)
        .current_dir(&dir)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "300")
        .output()
        .expect("loft runs");
    assert!(
        out.status.success(),
        "--emit-c:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        fa::read_to_string(dir.join("p.c")).is_ok_and(|c| c.contains("INT64_C(42)")),
        "the C carries the library's answer (`twice(21)`, folded)"
    );
    assert!(
        !fa::exists(lib.join("native-auto")),
        "a C build started a cargo build of the library's Rust crate"
    );
    // Run as well: the library path the program starts with is resolved without a build.
    if have_cc() && loft::platform::c_runtime_posix_host() {
        let run = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg("--native-c")
            .arg("--lib")
            .arg(dir.join("libs"))
            .arg(&prog)
            .current_dir(&dir)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "300")
            .output()
            .expect("loft runs");
        let err = String::from_utf8_lossy(&run.stderr);
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "42\n",
            "--native-c:\n{err}"
        );
        assert!(
            !err.contains("building native library"),
            "--native-c started a cargo build of the library's Rust crate:\n{err}"
        );
    }
    let _ = fa::remove_dir_all(&dir);
}
