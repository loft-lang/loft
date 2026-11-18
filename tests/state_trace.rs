// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `LOFT_STATE_TRACE=1` (`src/parser/state_trace.rs`): the lines the Rust front end prints for
//! one small function, pinned point by point — `stmt`, `stmt-end`, `var` (a variable created,
//! and one whose type a later statement set), `route`, `fn-end` — and nothing at all with the
//! switch off.  The loft-written elaborator prints the same lines; gate E compares the two and
//! checks on every run that neither side went silent (elab_gate.py `--pass1`).
//!
//! @falsified-at: PENDING-SHA — by the run that wrote this test.
//!   CHANNEL: the trace on stderr, line for line.
//!   ARMED: LOFT_STATE_TRACE=1 (set by the test).
//!   WITNESS: with one line of the expected list missing (the `route classify_ret_promotion`
//!   line the front end prints where the body ends), the test failed naming it: `line 18 of
//!   `twice`'s first pass`, left the `route` line, right the `fn-end … ret=` line.
//!   HOLDS: the program prints `[3,5]` with the switch on and off.

const PROG: &str = "fn twice(v: vector<integer>) -> vector<integer> {
  out = [];
  for x in v { out += [x * 2]; }
  out
}
fn main() {
  r = twice([1, 2]).map(fn(a: integer) -> integer { a + 1 });
  println(\"{r}\");
}
";

/// The first pass of `twice`, in the order printed.
const TWICE_PASS_1: &[&str] = &[
    "state 1 2:6 stmt",
    "state 1 2:11 route parse_vector",
    "state 1 2:6 stmt-end value=void",
    "state 1 2:6 var n_twice 0 v + vector<integer>",
    "state 1 2:6 var n_twice 1 __retbuf + vector<integer>",
    "state 1 2:6 var n_twice 2 out + unknown(0)",
    "state 1 3:6 stmt",
    "state 1 3:19 stmt expected=vector<int>",
    "state 1 3:25 route parse_vector",
    "state 1 3:19 stmt-end value=void",
    "state 1 3:19 var n_twice 2 out unknown(0) -> vector<integer>",
    "state 1 3:19 var n_twice 3 _vector_1 + vector<integer>[\"v\"]",
    "state 1 3:19 var n_twice 4 x#index + integer",
    "state 1 3:19 var n_twice 5 x + integer",
    "state 1 3:19 var n_twice 6 _elm_1 + ref(boolean)[\"out\"]",
    "state 1 3:6 stmt-end value=void",
    "state 1 4:6 stmt",
    "state 1 4:6 stmt-end value=vector<integer>[\"out\"]",
    // The return decision is taken when the body ends, with the lexer on the next line.
    "state 1 6:3 route classify_ret_promotion=Rename { buf_attr: 1, chain_site: false }",
    "state 1 1:32 fn-end n_twice ret=vector<integer>[\"__retbuf\"]",
    "state 1 1:32 fn-end n_twice var 0 v=vector<integer>",
    "state 1 1:32 fn-end n_twice var 1 __retbuf=vector<integer>",
    "state 1 1:32 fn-end n_twice var 2 out=vector<integer>",
    "state 1 1:32 fn-end n_twice var 3 _vector_1=vector<integer>[\"v\"]",
    "state 1 1:32 fn-end n_twice var 4 x#index=integer",
    "state 1 1:32 fn-end n_twice var 5 x=integer",
    "state 1 1:32 fn-end n_twice var 6 _elm_1=ref(boolean)[\"out\"]",
];

fn run(trace: bool) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("loft-state-trace-{}", std::process::id()));
    loft::file_access::create_dir_all(&dir).unwrap();
    let prog = dir.join(if trace { "on.loft" } else { "off.loft" });
    loft::file_access::write(&prog, PROG).unwrap();
    let mut cmd = loft::platform::process::harness_command(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_loft"
    )));
    cmd.args(["--interpret", "--no-warnings"])
        .arg(&prog)
        .env_remove("LOFT_STATE_TRACE");
    if trace {
        cmd.env("LOFT_STATE_TRACE", "1");
    }
    let o = cmd.output().expect("loft runs");
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
fn the_trace_prints_each_point_of_a_function_and_nothing_when_off() {
    let (out, err) = run(true);
    assert_eq!(out, "[3,5]\n", "{err}");
    let lines: Vec<&str> = err.lines().filter(|l| l.starts_with("state ")).collect();
    let at = lines
        .iter()
        .position(|l| *l == TWICE_PASS_1[0])
        .unwrap_or_else(|| panic!("no `{}` in:\n{err}", TWICE_PASS_1[0]));
    for (k, want) in TWICE_PASS_1.iter().enumerate() {
        assert_eq!(
            lines.get(at + k).copied(),
            Some(*want),
            "line {k} of `twice`'s first pass"
        );
    }
    // The second pass prints too, and the special route the `map` takes is named.
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("state 2 ") && l.contains(" fn-end n_twice ret="))
    );
    assert!(
        lines.iter().any(|l| l.ends_with(" route parse_map")),
        "{err}"
    );
    let (out, err) = run(false);
    assert_eq!(out, "[3,5]\n");
    assert!(
        !err.contains("state "),
        "the trace printed with the switch off: {err}"
    );
}
