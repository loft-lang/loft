// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! A vector literal's element type names no frame variable.  `v = [a, b]` of two record
//! locals copies each item into a slot of `v`'s own store, so the element borrows nothing the
//! frame holds — and the type is shared: the first literal of an element type registers the
//! `vector<T>` definition, where a frame variable's number means nothing (`introspect` printed
//! it as `["??"]`).  Values cannot see it; the definition's type can.
extern crate loft;

use loft::data::Type;
use loft::parser::Parser;

fn element_deps(src: &str, vector_def: &str) -> Vec<u16> {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse_str(src, "<probe>", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "probe must parse clean: {:?}",
        p.diagnostics.lines()
    );
    let d = p.data.def_nr(vector_def);
    assert_ne!(d, u32::MAX, "{vector_def} must be defined");
    match p.data.def(d).returned() {
        Type::Vector(elm, _) => elm.depend(),
        other => panic!("{vector_def} returns {other:?}, not a vector"),
    }
}

#[test]
fn a_literal_of_record_locals_registers_an_element_borrowing_nothing() {
    let deps = element_deps(
        "struct Ar { id: integer }\n\
         fn f() -> integer { a = Ar { id: 1 }; b = Ar { id: 2 }; v = [a, b]; v[1].id }\n\
         fn main() { assert(f() == 2, \"read\"); }",
        "vector<Ar>",
    );
    assert!(deps.is_empty(), "vector<Ar>'s element borrows {deps:?}");
}

/// A vector of vectors is ONE definition, named after its element type, and it takes the
/// store type.  `check_vector` named the definition after the element's DEFINITION, which for a
/// vector is the library's generic `vector`, so formatting `{g}` registered a second
/// `vector<vector>` (holding the store type) beside the real `vector<vector<integer>>`.
#[test]
fn a_nested_vector_has_one_definition_and_it_holds_the_store_type() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse_str(
        "fn main() { g = [[1, 2], [3]]; println(\"{g}\"); }",
        "<probe>",
        false,
    );
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "probe must parse clean: {:?}",
        p.diagnostics.lines()
    );
    assert_eq!(
        p.data.def_nr("vector<vector>"),
        u32::MAX,
        "no `vector<vector>` definition"
    );
    let d = p.data.def_nr("vector<vector<integer>>");
    assert_ne!(d, u32::MAX, "vector<vector<integer>> must be defined");
    assert_ne!(
        p.data.def(d).known_type,
        u16::MAX,
        "vector<vector<integer>> holds its store type"
    );
}
