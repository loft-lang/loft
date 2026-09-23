// Copyright (c) 2021-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Tests that require Rust-level slot inspection (.slots()).
// All other string tests have moved to tests/scripts/31-strings.loft.

extern crate loft;

mod testing;

use loft::data::Value;

/// Byte-indexed `text[i]` returns the character that contains the
/// byte at position `i`.  `"♥😃"` has 7 bytes (3 for ♥, 4 for 😃),
/// so `a[0..2]` returns `♥` (each byte of ♥) and `a[3..6]` returns
/// `😃` (each byte of 😃).  Out-of-range indices (`a[7]` and up) are a
/// RECOVERABLE fault (@P356): they return the null char and execution
/// continues; the opt-in `LOFT_DEV_SOFT_HALT` mode surfaces the
/// `IndexOutOfBounds` for debugging — covered in
/// `tests/runtime_errors.rs::kind_index_out_of_bounds_text_returns_null_and_continues`.
#[test]
fn utf8_index() {
    expr!("a=\"♥😃\"; a[0] + a[1] + a[2] + a[3] + a[4] + a[5] + a[6]")
        .result(Value::str("♥♥♥😃😃😃😃"));
}

#[test]
fn string_scope() {
    expr!(
        "
  a=1;
  b=\"\";
  for n in 1..4 {
    t=\"1\";
    b+=\"n\" + \":{n}\" + \"=\";
    for _m in 1..n {
      t+=\"2\";
    };
    b += t+\" \";
    a += t as integer ?? 0
  };
  \"{a} via {b}\"
"
    )
    // Slot expectation refreshed after P223 (work-text wrap on
    // self-referencing text assignments) added 2 extra `__work_*`
    // slots — the fix wraps `b += rhs-with-self-ref` and `t += "2"`
    // shapes in protective work-buffers so the interpreter's
    // clear-before-evaluate text-Set semantics don't destroy the
    // accumulator.  See `parser/expressions.rs:1273-1295`.
    // @PLAN53 — aligned layout: every slot step rounds up to 8.
    // @PLN25 DN3: `t as integer` now yields `integer?`; the `?? 0` discharge
    // adds a null-coalesce block (`ncc:14` / `__ncc_1`) at the tail.
    // @PLN157 P3b: both counted loops carry literal `lo`, so each drops its
    // null-test-and-choose ops (~4 apiece) and every later span starts earlier.
    // loft#1619 (I-For): the inner loop's end `n` is bound once to `_range_end_1`, so `n`
    // is last read there — its span ends at the bind and `__ncc_1` reuses its slot.
    .slots(
        "\
  block:1
  __work_5+24=8 [0..144]
  __work_4+24=32 [3..143]
  __work_3+24=56 [6..142]
  __work_2+24=80 [9..141]
  __work_1+24=104 [12..140]
  test_value+24=128 [15..139]
  │ block:2
  │ a+8=152 [17..101]
  │ b+24=160 [18..118]
  │ │ for:3
  │ │ n#index+8=184 [22..97]
  │ │ │ loop:4L [seq 23..98]
  │ │ │ n+8=192 [31..60]
  │ │ │ │ │ │ │ ncc:14
  │ │ │ │ │ │ │ __ncc_1+8=192 [90..93]
  │ │ │ │ block:6
  │ │ │ │ t+24=200 [32..97]
  │ │ │ │ │ for:9
  │ │ │ │ │ _range_end_1+8=224 [61..75]
  │ │ │ │ │ _m#index+8=232 [63..75]
  │ │ │ │ │ │ loop:10L [seq 64..76]
  │ │ │ │ │ │ _m+8=240 [72..72]",
    )
    .result(Value::str("136 via n:1=1 n:2=12 n:3=122 "));
}

#[test]
fn loop_variable() {
    expr!("a = 0; for _t in 1..5 { b = \"123\"; a += b as integer ?? 0; if a > 200 { break; }}; a")
        // @PLAN53 — aligned layout; `test_value` sorts last as its 8-rounded
        // slot lands above the loop body.
        // @PLN25 DN3: `b as integer` now yields `integer?`, so `?? 0` discharges it
        // — that adds a null-coalesce block (`ncc:7` / `__ncc_1`) to the layout.
        // @PLN157 P3b: a literal-`lo` counted loop initialises its counter to
        // `lo - 1` and increments unconditionally — the null-test-and-choose
        // ops are gone, so every span past the loop head starts ~4 ops earlier.
        .slots(
            "\
  block:1
  __work_1+24=8 [0..57]
  │ block:2
  │ a+8=32 [4..34]
  │ │ for:3
  │ │ _t#index+8=40 [6..33]
  │ │ │ loop:4L [seq 7..34]
  │ │ │ _t+8=48 [15..15]
  │ │ │ │ │ ncc:7
  │ │ │ │ │ __ncc_1+8=48 [22..25]
  │ │ │ │ block:6
  │ │ │ │ b+24=56 [16..33]
  test_value+8=80 [35..42]",
        )
        .result(Value::Int(246));
}
