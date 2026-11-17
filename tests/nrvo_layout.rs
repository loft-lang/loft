// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @P377 S1 — `nrvo_collapse_tail_set` hands a function's tail local to the call that binds it
//! (`v = mk(u); v` → `n_mk(u, v)`), so the callee builds straight into the caller's return
//! buffer.  Whether it does must not depend on a newline: the bind and the tail on two lines
//! put a line marker between the two statements, and the penultimate-statement test read that
//! marker as the statement — the same two statements on one line collapsed, on two lines they
//! kept a work ref and a copy.  Both layouts answer the same values, so only the lowering shows
//! the difference; this reads it.

extern crate loft;

use loft::data::{Data, Value};
use loft::parser::Parser;

fn parse(script: &str) -> Data {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(script, "nrvo_layout", false);
    assert!(
        p.diagnostics.is_empty(),
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    p.data
}

/// The name of the local `after` hands to its call of `callee` as the return buffer: the
/// call's last argument.
fn buffer_of(script: &str, callee: &str) -> String {
    let data = parse(script);
    let after = data.def_nr("n_after");
    let target = data.def_nr(callee);
    let def = data.def(after);
    let mut found = Vec::new();
    def.code().walk(&mut |v| {
        if let Value::Call(d, args) = v.unspan()
            && *d == target
            && let Some(Value::Var(b)) = args.last().map(Value::unspan)
        {
            found.push(def.variables.name(*b).to_string());
        }
    });
    assert_eq!(found.len(), 1, "one call of {callee} in n_after: {found:?}");
    found.remove(0)
}

const STRUCT_ONE_LINE: &str = "struct S { a: integer, b: integer }
fn mk(u: integer) -> S { S { a: u, b: 0 } }
fn after(u: integer) -> S { v = mk(u); v }
fn main() { t = after(1).a; assert(t == 1, \"one line\"); }";

const STRUCT_TWO_LINES: &str = "struct S { a: integer, b: integer }
fn mk(u: integer) -> S { S { a: u, b: 0 } }
fn after(u: integer) -> S {
  v = mk(u);
  v
}
fn main() { t = after(1).a; assert(t == 1, \"two lines\"); }";

const STRUCT_COMMENT_BETWEEN: &str = "struct S { a: integer, b: integer }
fn mk(u: integer) -> S { S { a: u, b: 0 } }
fn after(u: integer) -> S {
  v = mk(u);
  // nothing happens here

  v
}
fn main() { t = after(1).a; assert(t == 1, \"comment\"); }";

const ENUM_TWO_LINES: &str = "enum Ev { A { x: integer }, B { y: text } }
fn mk(u: integer) -> Ev { if u % 2 == 0 { A { x: u } } else { B { y: \"b\" } } }
fn after(u: integer) -> Ev {
  v = mk(u);
  v
}
fn main() { r = after(2); assert(r is A, \"enum\"); }";

/// A statement between the bind and the tail is a different shape: the fast path declines it.
const STATEMENT_BETWEEN: &str = "struct S { a: integer, b: integer }
fn mk(u: integer) -> S { S { a: u, b: 0 } }
fn after(u: integer) -> S {
  v = mk(u);
  w = u + 1;
  v.b = w;
  v
}
fn main() { t = after(1).b; assert(t == 2, \"between\"); }";

#[test]
fn a_tail_local_is_handed_to_its_bind_whatever_the_layout() {
    assert_eq!(buffer_of(STRUCT_ONE_LINE, "n_mk"), "v");
    assert_eq!(buffer_of(STRUCT_TWO_LINES, "n_mk"), "v", "two lines");
    assert_eq!(
        buffer_of(STRUCT_COMMENT_BETWEEN, "n_mk"),
        "v",
        "a comment and a blank line"
    );
    assert_eq!(
        buffer_of(ENUM_TWO_LINES, "n_mk"),
        "v",
        "a struct enum on two lines"
    );
}

#[test]
fn a_statement_between_the_bind_and_the_tail_keeps_the_work_ref() {
    assert!(buffer_of(STATEMENT_BETWEEN, "n_mk").starts_with("__ref_"));
}

const GROW: &str = "struct Canvas { data: vector<integer>, w: integer }
fn grow(c: const Canvas, extra: integer) -> Canvas {
  g = Canvas { data: [], w: c.w + 1 };
  for x in c.data { g.data += [x]; }
  for _k in 0..extra { g.data += [c.w]; }
  g
}
fn mk(t: integer) -> Canvas { Canvas { data: [t, t], w: t + 3 } }
";

/// A call that READS the tail local must not be handed it as its buffer: the callee would
/// build its answer into the record it reads (`cv = grow(cv, 2); cv` answered `1 2 1 0` for
/// `5 4 1 4`, silently, on both backends).  On one line and on two.
#[test]
fn a_call_reading_the_tail_local_keeps_its_work_ref() {
    let one =
        format!("{GROW}fn after(t: integer) -> Canvas {{ cv = mk(t); cv = grow(cv, 2); cv }}");
    let two = format!(
        "{GROW}fn after(t: integer) -> Canvas {{\n  cv = mk(t);\n  cv = grow(cv, 2);\n  cv\n}}"
    );
    assert!(buffer_of(&one, "n_grow").starts_with("__ref_"), "one line");
    assert!(buffer_of(&two, "n_grow").starts_with("__ref_"), "two lines");
}
