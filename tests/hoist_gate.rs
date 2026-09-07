// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#885 — the gate that decides whether a loop may read a vector through a header
//! derived once instead of per element.
//!
//! Getting this wrong in the permissive direction is a silent wrong read: the loop keeps
//! using a record and a length that a write has moved. So the cases below are paired —
//! every op that must count as WRITING has a reader beside it, and every loop shape that
//! must hoist has a twin that differs only in the write.
//!
//! The end-to-end half (does the emitted Rust actually change, and does the program still
//! answer the same on both backends) is `tests/scripts/885-vector-hoist.loft` plus the
//! emission assertions at the bottom of this file.

extern crate loft;

use loft::data::{Data, Value};
use loft::generation::hoist;
use loft::parser::Parser;
use std::collections::HashMap;

/// Parse `script` against the stdlib and hand back its `Data`.
fn parse(script: &str) -> Data {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(script, "hoist_gate", false);
    assert!(
        p.diagnostics.is_empty(),
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    p.data
}

fn writes(data: &Data, name: &str) -> bool {
    let d_nr = data.def_nr(name);
    assert!(d_nr != u32::MAX, "`{name}` is not defined");
    hoist::may_write_store(&Value::Call(d_nr, Vec::new()), data, &mut HashMap::new())
}

/// The verdict "this op cannot name a store" is read off its declared parameters, so an
/// op whose parameters are not where the analysis looks reads as taking none — and every
/// mutator turns into a reader. That is exactly how the first cut of this gate let
/// `v.remove(0)` through, so the parameter list is pinned here rather than assumed.
#[test]
fn native_op_parameters_are_visible() {
    let data = parse("fn main() { }");
    for (name, arity) in [
        ("OpAppendVector", 3),
        ("OpRemoveVector", 3),
        ("OpGetVector", 3),
        ("OpGetSingle", 2),
        ("OpAddInt", 2),
    ] {
        let d_nr = data.def_nr(name);
        assert!(d_nr != u32::MAX, "`{name}` is not defined");
        assert_eq!(
            hoist::parameters_declared(&data, d_nr),
            arity,
            "{name} declares {arity} parameters — if this reads 0, every op looks scalar \
             and the gate fails OPEN"
        );
    }
}

/// Every mutating op must read as writing — through the op itself AND through the stdlib
/// function that wraps it, which is the route a loop body actually takes.
#[test]
fn mutators_write_a_store() {
    let data = parse("fn main() { }");
    for name in [
        "OpAppendVector",
        "OpInsertVector",
        "OpRemoveVector",
        "OpClearVector",
        "OpAppendCopy",
        "OpNewRecord",
        "OpFinishRecord",
        "OpFreeRef",
        "OpDatabase",
        "OpSetInt",
        "OpSetSingle",
    ] {
        if data.def_nr(name) == u32::MAX {
            continue; // not in this build's stdlib
        }
        assert!(writes(&data, name), "{name} must count as writing a store");
    }
}

/// …and the pure reads and the arithmetic must not, or no loop ever hoists.
#[test]
fn readers_and_arithmetic_are_store_free() {
    let data = parse("fn main() { }");
    for name in [
        "OpGetVector",
        "OpGetVectorNullable",
        "OpLengthVector",
        "OpGetSingle",
        "OpGetInt",
        "OpAddInt",
        "OpMulSingle",
        "OpLeInt",
        // `len(v)` — the wrapper a `for i in 0..len(v)` loop calls, whose whole body is
        // `OpLengthVector`.  It only reads as store-free if the walk follows the call.
        "t_6vector_len",
    ] {
        assert!(!writes(&data, name), "{name} must count as store-free");
    }
}

/// A user function is transparent: what it does decides, not that it is a call.
#[test]
fn user_fn_is_followed() {
    let data = parse(
        "fn grow(t: vector<integer>, x: integer) { t += [x]; }\n\
         fn peek(t: vector<integer>, i: integer) -> integer { t[i] ?? 0 }\n\
         fn main() { }",
    );
    assert!(writes(&data, "n_grow"), "a callee that appends writes");
    assert!(
        !writes(&data, "n_peek"),
        "a callee that only indexes does not"
    );
}

/// The whole query, at the level the emitter asks it: which loops hoist.
///
/// `hoistable(script, fn_name)` returns the number of vectors the ONE loop in `fn_name`
/// may hoist, so a `0` cell says the gate declined and a `1` says it fired.
fn hoistable(script: &str, fn_name: &str) -> usize {
    hoistable_tier(script, fn_name, true)
}

/// Same query with the @PLN157 P4a tier explicit: `allow_in_place = false` is the
/// `LOFT_NO_WRITE_HOIST=1` classification, where any store write blocks.
fn hoistable_tier(script: &str, fn_name: &str, allow_in_place: bool) -> usize {
    let data = parse(script);
    let d_nr = data.def_nr(fn_name);
    assert!(d_nr != u32::MAX, "`{fn_name}` is not defined");
    let mut cache = HashMap::new();
    let mut total = 0;
    data.def(d_nr).code().any_node(&mut |n| {
        if let Value::Loop(body) = n {
            total += hoist::hoistable_vectors(body, &data, d_nr, &mut cache, allow_in_place).len();
        }
        false
    });
    total
}

const READ: &str = "fn f(v: vector<integer>, n: integer) -> integer {
  s = 0;
  for i in 0..n { s = s + (v[i] ?? 0); }
  s
}
fn main() { }";

#[test]
fn a_read_only_loop_hoists() {
    assert_eq!(hoistable(READ, "n_f"), 1);
}

/// Each of these differs from `READ` in exactly one thing: something in the loop can
/// reach the vector's record or the variable that names it.
#[test]
fn a_loop_that_can_reach_the_vector_declines() {
    for (label, body) in [
        ("append to it", "s = s + (v[i] ?? 0); v += [i];"),
        ("append to another", "s = s + (v[i] ?? 0); w += [i];"),
        ("remove from it", "s = s + (v[i] ?? 0); v.remove(0);"),
        ("clear it", "s = s + (v[i] ?? 0); v.clear();"),
        ("rebind it", "s = s + (v[i] ?? 0); v = w;"),
        ("write through an alias", "s = s + (v[i] ?? 0); u += [i];"),
        (
            "call something that writes",
            "s = s + (v[i] ?? 0); grow(v, i);",
        ),
        (
            "write in a nested loop",
            "for j in 0..1 { v += [j]; } s = s + (v[i] ?? 0);",
        ),
    ] {
        let script = format!(
            "fn grow(t: vector<integer>, x: integer) {{ t += [x]; }}
fn f(v: vector<integer>, n: integer) -> integer {{
  s = 0;
  w: vector<integer> = [];
  u = v;
  for i in 0..n {{ {body} }}
  s + len(w) + len(u)
}}
fn main() {{ }}"
        );
        assert_eq!(
            hoistable(&script, "n_f"),
            0,
            "a loop that can {label} must not hoist"
        );
    }
}

/// Stage 2 recognises `OpGet<scalar>(OpGetVector*(Var(v), size, i), fld)` as one load.
///
/// The emitter and the pre-eval collector both ask `fused_element_read`, and they have to
/// agree: if the collector hoists the inner read into a `let _pre_N` that the emitter then
/// folds away, the read happens twice. So what is pinned here is the SHAPE — which getters
/// qualify, and that a vector operand which is not a plain variable does not.
#[test]
fn the_fused_shape_is_recognised() {
    let data = parse("fn main() { }");
    let vec_read = |elem_op: &str| {
        Value::Call(
            data.def_nr(elem_op),
            vec![Value::Var(3), Value::Int(4), Value::Var(7)],
        )
    };
    let read = |getter: &str, inner: Value| {
        hoist::fused_element_read(&data, getter, &[inner, Value::Int(0)]).map(|f| f.path)
    };

    for getter in ["OpGetInt", "OpGetSingle", "OpGetFloat"] {
        for elem_op in ["OpGetVector", "OpGetVectorNullable"] {
            assert_eq!(
                read(getter, vec_read(elem_op)),
                Some((3, Vec::new())),
                "{getter} over {elem_op} must fuse, naming the vector variable"
            );
        }
    }

    // A getter whose body is not a plain typed load stays unfused rather than approximated.
    for getter in ["OpGetBoolean", "OpGetByte", "OpGetCharacter", "OpGetText"] {
        if data.def_nr(getter) == u32::MAX {
            continue;
        }
        assert!(
            read(getter, vec_read("OpGetVector")).is_none(),
            "{getter} has a different shape and must not fuse"
        );
    }

    // A vector operand that is an EXPRESSION has no header to read against, and re-emitting
    // it for the fallback would evaluate it twice.
    let nested = Value::Call(
        data.def_nr("OpGetVectorNullable"),
        vec![
            vec_read("OpGetVectorNullable"),
            Value::Int(4),
            Value::Var(7),
        ],
    );
    assert!(
        read("OpGetInt", nested).is_none(),
        "an indexed read of an indexed read must not fuse"
    );
}

/// A `par` arm and a `yield` run code this walk is not looking at.
#[test]
fn concurrency_and_suspension_decline() {
    let script = "fn f(v: vector<integer>, n: integer) -> iterator<integer> {
  for i in 0..n { yield v[i] ?? 0; }
}
fn main() { }";
    assert_eq!(
        hoistable(script, "n_f"),
        0,
        "a yielding loop must not hoist"
    );
}

/// @PLN157 P4a — the in-place tier: a loop whose only store writes are scalar
/// element SETS keeps its headers, because an in-place write moves no record and
/// changes no length (aliased or not); anything that grows, removes or rebinds
/// still declines, and `LOFT_NO_WRITE_HOIST=1` (`allow_in_place = false`)
/// restores the all-or-nothing gate.
#[test]
fn an_in_place_write_loop_keeps_its_headers() {
    let script = "fn f(v: vector<integer>, n: integer) -> integer {
  s = 0;
  for i in 0..n { v[i] = (v[i] ?? 0) + 1; s = s + (v[i] ?? 0); }
  s
}
fn main() { }";
    assert_eq!(
        hoistable_tier(script, "n_f", true),
        1,
        "an in-place element write must not decline the hoist"
    );
    assert_eq!(
        hoistable_tier(script, "n_f", false),
        0,
        "LOFT_NO_WRITE_HOIST restores the pre-P4a gate"
    );
}

#[test]
fn an_in_place_write_through_an_alias_keeps_both_headers() {
    let script = "fn f(v: vector<integer>, n: integer) -> integer {
  s = 0;
  u = v;
  for i in 0..n { u[i] = 1; s = s + (v[i] ?? 0); }
  s + len(u)
}
fn main() { }";
    assert_eq!(
        hoistable_tier(script, "n_f", true),
        2,
        "an in-place write cannot move a record, so even the alias's header stays valid"
    );
}

#[test]
fn a_growing_write_beside_an_in_place_one_still_declines() {
    let script = "fn f(v: vector<integer>, n: integer) -> integer {
  s = 0;
  for i in 0..n { v[i] = 1; v += [i]; s = s + (v[i] ?? 0); }
  s
}
fn main() { }";
    assert_eq!(
        hoistable_tier(script, "n_f", true),
        0,
        "a grow in the same body invalidates headers whatever else is in-place"
    );
}

/// @PLN157 P4b — the fused WRITE shape: which setters qualify, and that a vector
/// operand which is not a plain variable does not.  The emitter and the pre-eval
/// collector both ask `fused_element_write`, so the pinned shape is the agreement.
#[test]
fn the_fused_write_shape_is_recognised() {
    let data = parse("fn main() { }");
    let vec_addr = |elem_op: &str| {
        Value::Call(
            data.def_nr(elem_op),
            vec![Value::Var(3), Value::Int(4), Value::Var(7)],
        )
    };
    let write = |setter: &str, inner: Value| {
        hoist::fused_element_write(&data, setter, &[inner, Value::Int(0), Value::Int(9)])
            .map(|f| f.path)
    };

    for setter in ["OpSetInt", "OpSetSingle", "OpSetFloat"] {
        for elem_op in ["OpGetVector", "OpGetVectorNullable"] {
            assert_eq!(
                write(setter, vec_addr(elem_op)),
                Some((3, Vec::new())),
                "{setter} over {elem_op} must fuse, naming the vector variable"
            );
        }
    }

    // A setter that re-bases, masks or translates stays unfused rather than approximated.
    for setter in ["OpSetByte", "OpSetShort", "OpSetBoolean", "OpSetText"] {
        if data.def_nr(setter) == u32::MAX {
            continue;
        }
        assert!(
            write(setter, vec_addr("OpGetVector")).is_none(),
            "{setter} has a different shape and must not fuse"
        );
    }

    // A record-field write (`Var` target, no element address) keeps the template.
    assert!(
        hoist::fused_element_write(
            &data,
            "OpSetFloat",
            &[Value::Var(3), Value::Int(0), Value::Int(9)]
        )
        .is_none(),
        "a field write on a record is not an element write"
    );
}

/// @PLN157 P4d — a vector reached through CONST field offsets hoists and fuses:
/// the path key carries the offsets, and the collection, the recognisers and the
/// emission all key on it together.
#[test]
fn a_field_reached_vector_hoists_and_fuses() {
    let script = "struct L { x0: integer = 0, best: vector<float> }
fn f(lay: L, n: integer) -> float {
  s = 0.0;
  for i in 0..n { lay.best[i] = (lay.best[i] ?? 0.0) + 1.5; s += lay.best[i] ?? 0.0; }
  s
}
fn main() { }";
    assert_eq!(
        hoistable_tier(script, "n_f", true),
        1,
        "a field-reached vector is one hoist candidate"
    );

    // The recogniser names the path: root var + the const offset chain.
    let data = parse(script);
    let field = Value::Call(
        data.def_nr("OpGetField"),
        vec![Value::Var(0), Value::Int(8), Value::Int(81)],
    );
    let inner = Value::Call(
        data.def_nr("OpGetVector"),
        vec![field, Value::Int(8), Value::Var(7)],
    );
    assert_eq!(
        hoist::fused_element_write(&data, "OpSetFloat", &[inner, Value::Int(0), Value::Int(9)])
            .map(|f| f.path),
        Some((0, vec![8])),
        "the write recogniser carries the field path"
    );
}
