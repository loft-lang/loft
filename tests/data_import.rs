// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Unit tests for the P173 Data helpers: `import_all_overwrite`,
//! `import_name_overwrite`, and `rewrite_unknown_refs`.
//!
//! These operate directly on `Data` without going through the parser, so
//! they're purely table-driven and independent of lexer / file I/O.

extern crate loft;

use loft::data::{Data, DefType, IntegerSpec, Position, Type};

fn pos() -> Position {
    Position {
        file: "unit".into(),
        line: 1,
        pos: 1,
    }
}

/// Register a pub definition in `src` with the given name and return its def_nr.
fn add_pub(data: &mut Data, src: u16, name: &str, def_type: DefType) -> u32 {
    data.source = src;
    let is_struct = matches!(def_type, DefType::Struct);
    let d_nr = data.add_def(name, &pos(), def_type);
    data.definitions[d_nr as usize].pub_visible = true;
    if is_struct {
        // Mirror what typedef::actual_types does: struct returned type points
        // to itself via Type::Reference.
        data.definitions[d_nr as usize].returned = Type::Reference(d_nr, loft::data::Deps::none());
    }
    d_nr
}

/// Register an Unknown stub in `src` with the given name; return its def_nr.
fn add_stub(data: &mut Data, src: u16, name: &str) -> u32 {
    data.source = src;
    data.add_def(name, &pos(), DefType::Unknown)
}

/// `import_all_overwrite` replaces Unknown stubs in the target source with
/// the real def from the lib source.
#[test]
fn overwrite_replaces_unknown_stub() {
    let mut d = Data::new();
    // Real Player struct in source 1 (pub).
    let real_player = add_pub(&mut d, 1, "Player", DefType::Struct);
    // Unknown stub for Player in source 2 (as if a forward reference from
    // a cyclic use in source 2 encountered Player before source 1 was parsed).
    let stub = add_stub(&mut d, 2, "Player");
    assert_ne!(real_player, stub);

    d.import_all_overwrite(1, 2);

    // After overwrite, source-2 lookup for Player should point to the real def.
    let looked_up = d.source_nr(2, "Player");
    assert_eq!(
        looked_up, real_player,
        "source-2 Player should now resolve to real def"
    );
}

/// `import_all_overwrite` preserves real local definitions (local wins).
#[test]
fn overwrite_preserves_real_local_def() {
    let mut d = Data::new();
    let real_in_1 = add_pub(&mut d, 1, "Player", DefType::Struct);
    let real_in_2 = add_pub(&mut d, 2, "Player", DefType::Struct);

    d.import_all_overwrite(1, 2);

    // source-2 lookup still returns source-2's own Player, not source-1's.
    let looked_up = d.source_nr(2, "Player");
    assert_eq!(looked_up, real_in_2);
    assert_ne!(looked_up, real_in_1);
}

/// `import_all_overwrite` inserts when there is no prior binding.
#[test]
fn overwrite_inserts_when_missing() {
    let mut d = Data::new();
    let real = add_pub(&mut d, 1, "Player", DefType::Struct);

    d.import_all_overwrite(1, 2);

    let looked_up = d.source_nr(2, "Player");
    assert_eq!(looked_up, real);
}

/// `import_name_overwrite` replaces only the named stub, not other stubs.
#[test]
fn name_overwrite_targets_single_name() {
    let mut d = Data::new();
    let real_player = add_pub(&mut d, 1, "Player", DefType::Struct);
    add_pub(&mut d, 1, "Monster", DefType::Struct);
    let player_stub = add_stub(&mut d, 2, "Player");
    let monster_stub = add_stub(&mut d, 2, "Monster");

    let ok = d.import_name_overwrite(1, 2, "Player", "Player");
    assert!(ok);

    assert_eq!(d.source_nr(2, "Player"), real_player);
    // Monster was not imported, so its binding still points to the stub.
    assert_eq!(d.source_nr(2, "Monster"), monster_stub);
    // Stub def_nrs are still reachable via their own source (the stubs
    // themselves live in `definitions[]`).  We just assert the binding now
    // points to the real def, not the stub.
    assert_ne!(d.source_nr(2, "Player"), player_stub);
}

/// `rewrite_unknown_refs` patches Type::Unknown(stub) in a function's
/// returned type to the real type.
#[test]
fn rewrite_replaces_plain_unknown_returned() {
    let mut d = Data::new();
    let real_player = add_pub(&mut d, 1, "Player", DefType::Struct);
    let stub = add_stub(&mut d, 2, "Player");
    // Register a function in source 2 that returns Type::Unknown(stub).
    d.source = 2;
    let fn_nr = d.add_def("make_player", &pos(), DefType::Function);
    d.definitions[fn_nr as usize].returned = Type::Unknown(stub);

    d.rewrite_unknown_refs(stub, real_player);

    let new_ret = &d.definitions[fn_nr as usize].returned;
    assert!(
        matches!(new_ret, Type::Reference(def, _) if *def == real_player),
        "returned type should be Reference(real_player); got {new_ret:?}"
    );
}

/// `rewrite_unknown_refs` patches Vector<Unknown(stub)> to Vector<Reference>.
#[test]
fn rewrite_replaces_vector_of_unknown() {
    let mut d = Data::new();
    let real_player = add_pub(&mut d, 1, "Player", DefType::Struct);
    let stub = add_stub(&mut d, 2, "Player");
    d.source = 2;
    let fn_nr = d.add_def("players", &pos(), DefType::Function);
    d.definitions[fn_nr as usize].returned =
        Type::Vector(Box::new(Type::Unknown(stub)), loft::data::Deps::none());

    d.rewrite_unknown_refs(stub, real_player);

    let new_ret = d.definitions[fn_nr as usize].returned.clone();
    match new_ret {
        Type::Vector(inner, _) => {
            assert!(
                matches!(*inner, Type::Reference(def, _) if def == real_player),
                "inner should be Reference(real_player); got {inner:?}"
            );
        }
        other => panic!("expected Vector, got {other:?}"),
    }
}

/// `rewrite_unknown_refs` leaves unrelated types alone.
#[test]
fn rewrite_leaves_unrelated_alone() {
    let mut d = Data::new();
    let real_player = add_pub(&mut d, 1, "Player", DefType::Struct);
    let stub = add_stub(&mut d, 2, "Player");
    d.source = 2;
    let fn_nr = d.add_def("count", &pos(), DefType::Function);
    d.definitions[fn_nr as usize].returned = Type::Integer(IntegerSpec {
        min: 0,
        max: 0,
        not_null: true,
        forced_size: None,
    });

    d.rewrite_unknown_refs(stub, real_player);

    assert!(matches!(
        d.definitions[fn_nr as usize].returned,
        Type::Integer(_)
    ));
}

/// `rewrite_unknown_refs` patches tuple element types.
#[test]
fn rewrite_replaces_tuple_element() {
    let mut d = Data::new();
    let real_player = add_pub(&mut d, 1, "Player", DefType::Struct);
    let stub = add_stub(&mut d, 2, "Player");
    d.source = 2;
    let fn_nr = d.add_def("pair", &pos(), DefType::Function);
    d.definitions[fn_nr as usize].returned = Type::Tuple(vec![
        Type::Integer(IntegerSpec {
            min: 0,
            max: 0,
            not_null: true,
            forced_size: None,
        }),
        Type::Unknown(stub),
    ]);

    d.rewrite_unknown_refs(stub, real_player);

    match d.definitions[fn_nr as usize].returned.clone() {
        Type::Tuple(elems) => {
            assert_eq!(elems.len(), 2);
            assert!(matches!(elems[0], Type::Integer(_)));
            assert!(matches!(elems[1], Type::Reference(def, _) if def == real_player));
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
}
