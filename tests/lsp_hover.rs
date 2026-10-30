// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 S5 — the loft-side hover accessor (`loft::lsp::symbol_at`) that drives
// `textDocument/hover`.  It resolves the identifier under a cursor to its
// definition and returns the clean signature plus the `///` doc block read from
// the definition's own source — the user buffer for local defs, the stdlib /
// library source on disk for imported ones (loft keeps no doc field; docs live
// in `.loft` source, the same convention `gendoc` reads).

use loft::lsp::{Hover, symbol_at};

/// Hover at a 1-based (line, col) in `buf`, with the repo-relative stdlib.
fn hover(buf: &str, line: u32, col: u32) -> Option<Hover> {
    symbol_at(buf, "buf.loft", "default", line, col)
}

#[test]
fn resolves_a_user_function_at_its_call_site_with_signature_and_doc() {
    let src = "\
/// Area of a rectangle.
/// width times height.
fn area(w: integer, h: integer) -> integer {
  w * h
}
fn main() {
  print(area(2, 3))
}
";
    // Hover `area` inside main's CALL (line 7) — resolves to the definition.
    let h = hover(src, 7, 10).expect("the call resolves to `area`");
    assert_eq!(h.signature, "fn area(w: integer, h: integer) -> integer");
    assert_eq!(
        h.doc,
        vec![
            "Area of a rectangle.".to_string(),
            "width times height.".to_string()
        ],
        "the `///` block above the definition, in reading order"
    );
    assert_eq!(h.def_file, "buf.loft");
    assert_eq!(h.def_line, 3, "points at the definition, not the call");
}

#[test]
fn resolves_a_stdlib_type_with_its_doc_read_from_stdlib_source() {
    // `StackFrame` is defined in `default/04_stacktrace.loft`; its signature comes
    // from the parsed data and its doc from that FILE — the cross-file read.
    let h = hover("fn main() {\n  x = StackFrame;\n}\n", 2, 8).expect("`StackFrame` resolves");
    assert!(
        h.signature.starts_with("struct StackFrame {"),
        "struct signature: {}",
        h.signature
    );
    assert_eq!(
        h.doc,
        vec!["One call frame in the stack trace.".to_string()],
        "the doc is read from the stdlib source file, not the buffer"
    );
    // Native separators on Windows — see the note in tests/lsp_cli.rs.
    assert!(
        h.def_file.replace('\\', "/").starts_with("default/"),
        "resolves into the stdlib source: {}",
        h.def_file
    );
}

#[test]
fn resolves_a_user_struct_signature() {
    let src = "struct Point {\n  x: integer,\n  y: integer,\n}\nfn main() {\n  p = Point { x: 1, y: 2 };\n}\n";
    // Hover `Point` at its USE on line 6.
    let h = hover(src, 6, 8).expect("`Point` resolves");
    assert_eq!(h.signature, "struct Point { x: integer, y: integer }");
    assert!(h.doc.is_empty(), "no doc above this struct");
    assert_eq!(h.def_line, 1);
}

#[test]
fn a_cursor_not_on_an_identifier_is_none() {
    let src = "fn main() {\n  a * b\n}\n";
    // Column 5 on line 2 is the `*`.
    assert!(hover(src, 2, 5).is_none(), "no symbol on an operator");
}

#[test]
fn an_unknown_identifier_is_none() {
    let src = "fn main() {\n  frobnicate\n}\n";
    assert!(
        hover(src, 2, 4).is_none(),
        "an undefined name resolves to no hover"
    );
}

/// `@FR-L-OutOfLine` (@C159, step 8) — a struct holding a field of its own type shows that
/// field as the author wrote it, `Tree?`: its out-of-line layout (`child_rec<Tree>`) is a
/// storage fact, never shown.
#[test]
fn a_recursive_field_shows_as_its_declared_type() {
    let src = "\
struct Tree {
  key: integer,
  left: Tree?,
}
fn main() {
  t = Tree { key: 1, left: Tree { key: 2 } };
  print(t.left.key)
}
";
    let h = hover(src, 6, 8).expect("the struct `Tree` resolves");
    assert_eq!(
        h.signature, "struct Tree { key: integer, left: Tree? }",
        "the declared field type, not the layout row"
    );
}
