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
    let p = parsed("fn up(n: integer) -> iterator<integer> { for i in 0..n { yield i; } }\n");
    let d = p.data.def_nr("n_up");
    let refusal =
        loft::c::emit_unit(&p.data, &p.database, &[d]).expect_err("a generator is not carried yet");
    assert_eq!(refusal.function, "up");
    assert!(
        refusal.why.contains("returns"),
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
