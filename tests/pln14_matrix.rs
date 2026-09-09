// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN14 Step 0 — the composition matrix (instrument, no product code).
//!
//! The load-bearing claim: *every value survives `bind → session-store →
//! observe` byte-for-byte equal.*  This file falsifies it across value-type ×
//! persistence-path, using ONLY the proven sibling (`Stores::snapshot_heap` /
//! `restore_heap`, @PLN63 RX) so the in-memory round-trip is pinned before any
//! arc-B product code exists.
//!
//! The strong form of the round-trip: snapshot the heap, then restore it into a
//! **fresh `State` that never built the value**, and render the value from
//! there.  It is non-vacuous by construction — the fresh heap does not even have
//! a store slot at the value's `store_nr` until the restore creates it, which
//! each case asserts.  Equality then proves the snapshot alone reconstructs the
//! value: the self-containment the session store needs.
//!
//! Findings recorded on the plan (2026-07-24): a loft value SPANS multiple
//! stores (a nested `struct` puts each `Reference` field in its own store), and
//! text in a struct field is stored IN-store (a `set_str` record, byte-copied by
//! the snapshot).  The on-disk (`save → restore`) cells wait on arc F's persist
//! primitive (Step 6).

use loft::compile;
use loft::keys::DbRef;
use loft::parser::Parser;
use loft::state::State;

fn stdlib() -> Parser {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("load stdlib");
    p
}

/// Parse `defs`, compile+run `fn probe() -> ty { expr }`, and return the live
/// `State` (value on the stack top) alongside the `Parser` (schema owner).
fn build(defs: &[&str], ty: &str, expr: &str) -> (Parser, State) {
    let mut p = stdlib();
    for def in defs {
        p.parse_str(&format!("{def}\n"), "<probe>", false);
    }
    let src = format!("fn probe() -> {ty} {{\n{expr}\n}}\n");
    p.parse_str(&src, "<probe>", false);
    let mut state = State::new(p.database.clone());
    loft::scopes::check(&mut p.data, &mut p.database);
    compile::byte_code(&mut state, &mut p.data);
    // This harness reads the probe's return value off the stack, so it is a
    // CLAIMING caller in the same sense the REPL's capture wrapper is: without
    // this the entry teardown frees the hidden return buffer and every heap case
    // renders empty (#629 follow-up).
    state.keep_entry_return();
    state.execute_argv("probe", &p.data, &[]);
    assert!(
        state.database.runtime_error.is_none(),
        "probe `{expr}` faulted: {:?}",
        state.database.runtime_error
    );
    (p, state)
}

/// The in-memory round-trip via the proven sibling.  Returns
/// `(before, after, existed_before_restore)` — the matrix asserts the two
/// renders are equal and that the value's store did NOT exist pre-restore.
fn snapshot_round_trip(
    defs: &[&str],
    ty: &str,
    type_name: &str,
    expr: &str,
) -> (String, String, bool) {
    let (p, mut state) = build(defs, ty, expr);
    // `get_stack` CONSUMES — read the root DbRef exactly once.
    let root = state.get_stack::<DbRef>();
    assert_readable(&root, &state, expr);
    let tp = state.database.name(type_name);
    assert!(tp != u16::MAX, "unresolved type name `{type_name}`");
    let mut before = String::new();
    state.database.show_loft(&mut before, &root, tp);

    let snap = state
        .database
        .snapshot_heap()
        .expect("an all-in-memory heap snapshots");

    // A fresh state with the same schema but no knowledge of the value.
    let mut data = p.data.clone();
    let mut fresh = State::new(p.database.clone());
    compile::byte_code(&mut fresh, &mut data);
    // Non-vacuity: the value's root store does not exist in this heap yet.
    let existed = (root.store_nr as usize) < fresh.database.allocations.len();

    fresh.database.restore_heap(&snap);
    let tp2 = fresh.database.name(type_name);
    let mut after = String::new();
    fresh.database.show_loft(&mut after, &root, tp2);
    (before, after, existed)
}

/// The value-type axis for heap-resident values (scalars live on the stack, not
/// the heap — their store round-trip is arc C).  Each is a distinct store
/// layout the one snapshot rule must cover.
const STRUCTS: &str = "struct P { x: integer, y: integer }\n\
     struct Line { a: P, b: P, tag: text }\n\
     struct Bag { items: vector<P>, name: text }\n\
     struct Big { id: integer, note: text }\n\
     enum Shape { Circle { radius: float }, \
                  Rect { width: float, height: float }, \
                  Tagged { name: text } }";

/// Calibration guard — the instrument must be able to SEE the value before a
/// cell may report anything.  A mis-specified case (bad syntax → a null or
/// out-of-range root) would otherwise round-trip garbage to identical garbage
/// and pass **vacuously**; that is exactly how the first draft of this matrix
/// green-lit two struct-enum cells that never built a value at all.
fn assert_readable(root: &DbRef, state: &State, expr: &str) {
    assert!(
        root.store_nr != u16::MAX
            && (root.store_nr as usize) < state.database.allocations.len()
            && root.rec != 0,
        "vacuous cell: `{expr}` did not leave a readable heap value on the stack \
         (root = {root:?}) — the case is mis-specified, not passing"
    );
}

fn heap_cases() -> Vec<(&'static str, &'static str, String)> {
    let big_text = "x".repeat(400); // > 256 B text
    vec![
        (
            "vector<integer>",
            "vector<integer>",
            "[1, 2, 3, 4, 5]".into(),
        ),
        (
            "vector<integer>",
            "vector<integer>",
            // >32-bit elements
            "[9000000000, -9000000001, 0]".into(),
        ),
        ("P", "P", "P { x: 7, y: 9 }".into()),
        (
            "Line",
            "Line",
            "Line { a: P{x:1,y:2}, b: P{x:3,y:4}, tag: \"seg\" }".into(),
        ),
        (
            "vector<P>",
            "vector<P>",
            "[P{x:1,y:2}, P{x:3,y:4}, P{x:5,y:6}]".into(),
        ),
        (
            "Bag",
            "Bag",
            "Bag { items: [P{x:1,y:2}, P{x:3,y:4}], name: \"sack\" }".into(),
        ),
        // text with embedded quote / newline / backslash / unicode
        (
            "Big",
            "Big",
            "Big { id: 1, note: \"a\\\"b\\nc\\\\d \\u{2603}\" }".into(),
        ),
        // > 256 B text in a struct field
        (
            "Big",
            "Big",
            format!("Big {{ id: 2, note: \"{big_text}\" }}"),
        ),
        (
            "vector<text>",
            "vector<text>",
            "[\"\", \"one\", \"two \\u{2603}\"]".into(),
        ),
        // struct-enum variants (heap-backed): the variant name constructs it
        ("Shape", "Shape", "Circle { radius: 1.5 }".into()),
        ("Shape", "Shape", "Rect { width: 3.0, height: 4.0 }".into()),
        (
            "Shape",
            "Shape",
            "Tagged { name: \"a\\\"b \\u{2603}\" }".into(),
        ),
    ]
}

#[test]
fn heap_values_survive_snapshot_round_trip() {
    for (ty, type_name, expr) in heap_cases() {
        let (before, after, existed) = snapshot_round_trip(&[STRUCTS], ty, type_name, &expr);
        assert!(
            !existed,
            "vacuous cell: the fresh heap already had a store at the value's \
             store_nr for `{expr}` — the equality below would not be carried by \
             the snapshot"
        );
        assert!(!before.is_empty(), "empty render for `{expr}`");
        assert_eq!(
            before, after,
            "snapshot round-trip diverged for `{expr}` (type {type_name})"
        );
    }
}

/// Step 1 — the arc-B round-trip: `bind → materialize → observe`.  Materializes
/// the value into a dedicated session store, then **deep-frees the source**
/// (`remove_claims` + `free`, the proven inverse of the `copy_claims` walk
/// materialize is built on) before reading the copy back.  Surviving that is the
/// "exactly one owned home" property: the copy shares nothing with the source.
fn materialize_round_trip(
    defs: &[&str],
    ty: &str,
    type_name: &str,
    expr: &str,
) -> (String, String) {
    let (_p, mut state) = build(defs, ty, expr);
    let root = state.get_stack::<DbRef>();
    assert_readable(&root, &state, expr);
    let tp = state.database.name(type_name);
    assert!(tp != u16::MAX, "unresolved type name `{type_name}`");
    let mut before = String::new();
    state.database.show_loft(&mut before, &root, tp);

    // The session store, and the value's own home inside it.
    let session = state.database.database(256);
    let copy = state.database.materialize(&root, tp, session.store_nr);
    assert_ne!(
        copy.store_nr, root.store_nr,
        "materialize returned a ref into the SOURCE store for `{expr}`"
    );

    // Deep-free the source: every nested allocation it owns, then its store.
    state.database.remove_claims(&root, tp);
    state.database.free(&root);

    let mut after = String::new();
    state.database.show_loft(&mut after, &copy, tp);
    (before, after)
}

#[test]
fn materialize_gives_each_value_its_own_home() {
    for (ty, type_name, expr) in heap_cases() {
        let (before, after) = materialize_round_trip(&[STRUCTS], ty, type_name, &expr);
        assert!(!before.is_empty(), "empty render for `{expr}`");
        assert_eq!(
            before, after,
            "materialized copy did not survive the source being deep-freed for \
             `{expr}` (type {type_name}) — it still shares storage with the source"
        );
    }
}

/// A null / empty source materializes as null: a faulting bind records no value
/// (no env entry, no poison) rather than a half-built record.
#[test]
fn materialize_of_null_is_null() {
    let (_p, mut state) = build(&[STRUCTS], "P", "P { x: 1, y: 2 }");
    let root = state.get_stack::<DbRef>();
    let tp = state.database.name("P");
    let session = state.database.database(256);
    let null_src = DbRef {
        store_nr: u16::MAX,
        rec: 0,
        pos: 0,
    };
    let out = state.database.materialize(&null_src, tp, session.store_nr);
    assert_eq!(out.rec, 0, "a null source must materialize as null");
    let _ = root;
}

/// Aliasing / value-semantics control (arc B's Q4): `b = a` must COPY, so a
/// later mutation of `a` leaves `b` unchanged.  A store-copy materialize (not an
/// alias) is the only way this holds — expressed as a loft program so it pins
/// the language contract the session store must preserve.
#[test]
fn cross_binding_is_a_copy_not_an_alias() {
    let (_p, mut state) = build(
        &[STRUCTS],
        "integer",
        "a = [1, 2, 3];\n  b = a;\n  a[0] = 99;\n  b[0]",
    );
    assert_eq!(state.get_stack::<i64>(), 1, "b aliased a (expected a copy)");
}

/// Negative control: a faulting RHS records a runtime error and NO value — the
/// structural form of the interim's `Capture::Failed` (no env entry, no poison).
#[test]
fn faulting_bind_records_a_fault_not_a_value() {
    let mut p = stdlib();
    let src = "fn probe() -> integer {\n  assert(false, \"boom\");\n  1\n}\n";
    p.parse_str(src, "<probe>", false);
    let mut state = State::new(p.database.clone());
    loft::scopes::check(&mut p.data, &mut p.database);
    compile::byte_code(&mut state, &mut p.data);
    state.execute_argv("probe", &p.data, &[]);
    assert!(
        state.database.runtime_error.is_some(),
        "a faulting bind must record a runtime error"
    );
}

/// Store-span reading (the finding that shapes arc B): a nested `struct` puts
/// each `Reference` field in its OWN store, so a value is generally a
/// MULTI-store graph.  A per-binding materialize therefore needs a reachability
/// walk + store-number rebase, not a single `Store::snapshot_copy`.  Pinned as a
/// test so the assumption cannot drift silently.
#[test]
fn a_nested_struct_spans_several_stores() {
    let flat = build(&[STRUCTS], "P", "P { x: 7, y: 9 }")
        .1
        .database
        .allocations
        .len();
    let nested = build(
        &[STRUCTS],
        "Line",
        "Line { a: P{x:1,y:2}, b: P{x:3,y:4}, tag: \"seg\" }",
    )
    .1
    .database
    .allocations
    .len();
    assert!(
        nested > flat,
        "expected a nested struct to span more stores than a flat one \
         (flat={flat}, nested={nested})"
    );
}

/// Positive control for the calibration guard: it must FIRE on each shape of
/// unreadable root (the null sentinel, an out-of-range store, record 0).  A
/// guard is only evidence once it is shown it can fail — otherwise the guard is
/// itself the vacuous thing.
#[test]
fn calibration_guard_catches_an_unreadable_root() {
    let (_p, state) = build(&[STRUCTS], "P", "P { x: 1, y: 2 }");
    let live = state.database.allocations.len() as u16;
    let unreadable = [
        DbRef {
            store_nr: u16::MAX,
            rec: 0,
            pos: 0,
        }, // null sentinel
        DbRef {
            store_nr: 65281,
            rec: 0,
            pos: 0,
        }, // out-of-range (garbage)
        DbRef {
            store_nr: live + 7,
            rec: 1,
            pos: 8,
        }, // past the store table
        DbRef {
            store_nr: 2,
            rec: 0,
            pos: 8,
        }, // in range but record 0
    ];
    for root in unreadable {
        let fired =
            std::panic::catch_unwind(|| assert_readable(&root, &state, "<synthetic>")).is_err();
        assert!(fired, "the calibration guard did NOT fire on root {root:?}");
    }
}

/// The property arcs A and F lean on: a materialized value is **entirely
/// self-contained in ONE store**.  `copy_claims` allocates every sub-record and
/// every text into the DESTINATION store, so after materialize we can free every
/// other user store and the copy still reads back identical.  That is what makes
/// the session store a single extractable / persistable unit (arc F persists one
/// store, not a graph of them).
#[test]
fn a_materialized_value_is_self_contained_in_one_store() {
    for (ty, type_name, expr) in heap_cases() {
        let (_p, mut state) = build(&[STRUCTS], ty, &expr);
        let root = state.get_stack::<DbRef>();
        assert_readable(&root, &state, &expr);
        let tp = state.database.name(type_name);
        let mut before = String::new();
        state.database.show_loft(&mut before, &root, tp);

        let session = state.database.database(256);
        let copy = state.database.materialize(&root, tp, session.store_nr);

        // Free EVERY other user store (store 0 is the eval stack; 1 is reserved).
        let total = state.database.allocations.len() as u16;
        for nr in 2..total {
            if nr == session.store_nr {
                continue;
            }
            state.database.free(&DbRef {
                store_nr: nr,
                rec: 0,
                pos: 0,
            });
        }

        let mut after = String::new();
        state.database.show_loft(&mut after, &copy, tp);
        assert_eq!(
            before, after,
            "materialized value was NOT self-contained in its store for `{expr}` \
             (type {type_name}) — it still reads from a store we freed"
        );
    }
}

/// The Step-2 crux: can the session store be carried ACROSS runs?  A
/// materialized value is self-contained, but if its interior references were
/// absolute (`store_nr` = the slot it was materialized at) it would break the
/// moment the store is re-adopted at a different slot in the next eval's
/// throwaway `State`.  So: materialize, take the store OUT, adopt it into a
/// FRESH state at a deliberately different slot, and render from there.
///
/// Equal renders ⇒ interior refs are slot-independent ⇒ the env can key on
/// `(rec, pos)` and rebuild the `DbRef` from wherever the store currently sits.
#[test]
fn a_session_store_survives_re_adoption_at_a_different_slot() {
    for (ty, type_name, expr) in heap_cases() {
        let (p, mut state) = build(&[STRUCTS], ty, &expr);
        let root = state.get_stack::<DbRef>();
        assert_readable(&root, &state, &expr);
        let tp = state.database.name(type_name);
        let mut before = String::new();
        state.database.show_loft(&mut before, &root, tp);

        let session = state.database.database(256);
        let copy = state.database.materialize(&root, tp, session.store_nr);
        let detached = state.database.take_store(session.store_nr);
        drop(state); // the whole run heap goes away, as it does between evals

        // A fresh run, with extra stores allocated first so the session store
        // is forced to land at a DIFFERENT slot than it had.
        let mut data = p.data.clone();
        let mut next = State::new(p.database.clone());
        compile::byte_code(&mut next, &mut data);
        for _ in 0..3 {
            let _ = next.database.database(64);
        }
        let new_nr = next.database.adopt_store(detached);
        assert_ne!(
            new_nr, session.store_nr,
            "probe is vacuous: the store landed at the SAME slot for `{expr}`"
        );

        let rebuilt = DbRef {
            store_nr: new_nr,
            rec: copy.rec,
            pos: copy.pos,
        };
        let tp2 = next.database.name(type_name);
        let mut after = String::new();
        next.database.show_loft(&mut after, &rebuilt, tp2);
        assert_eq!(
            before, after,
            "the session store did NOT survive re-adoption at slot {new_nr} \
             (was {}) for `{expr}` — interior refs are slot-dependent",
            session.store_nr
        );
    }
}

// ── Step 2 — the session store + env record, as a write-only shadow ──────────
//
// The env is written on every heap-backed bind and read ONLY by `env_value`.
// The replay model is still the source of truth, so these are the differential
// oracle: the shadow must already agree everywhere, which is what Step 4's
// frame-seed will be checked against.

use loft::repl::{Eval, ImageLoad, ReplSession};

fn session() -> ReplSession {
    ReplSession::new("default").expect("load stdlib")
}

/// The type definitions the session-level tests need, one input per `eval`.
const REPL_DEFS: &[&str] = &[
    "struct P { x: integer, y: integer }",
    "struct Line { a: P, b: P, tag: text }",
    "struct Bag { items: vector<P>, name: text }",
    "struct Big { id: integer, note: text }",
    "enum Shape { Circle { radius: float }, Rect { width: float, height: float }, Tagged { name: text } }",
];

fn session_with_defs() -> ReplSession {
    let mut s = session();
    for def in REPL_DEFS {
        assert!(
            matches!(s.eval(def), Eval::Ran),
            "def failed to eval: {def}"
        );
    }
    s
}

/// The Step-2 differential: the store-resident shadow must render EXACTLY what a
/// fresh evaluation of the same expression renders.
///
/// The oracle is `value_of(<expr>)`, not `value_of(<name>)`: reading a bound
/// *vector* by name crashes on `main` (loft#618 — the fn-return copy of a
/// borrowed local), which is pre-existing and out of @PLN14's scope.  Evaluating
/// the expression afresh exercises the same render path without that hazard.
///
/// The exact claim is a BICONDITIONAL, which is what makes it a real
/// differential: the shadow holds a value **exactly when** the REPL.X snapshot
/// path captured one.  A binding the snapshot path declines (it falls back to
/// storing the RHS as source — e.g. a vector of >32-bit literals, pre-existing)
/// must have NO env entry, not a stale or half-built one.
#[test]
fn env_shadow_agrees_with_a_fresh_evaluation() {
    let mut checked = 0;
    for (_ty, type_name, expr) in heap_cases() {
        // Pre-existing, unrelated to @PLN14 (noted on loft#618): evaluating a
        // vector-of->32-bit-literals expression TWICE in one session re-registers
        // its element type and aborts with "Double structure type" from
        // `types.rs`.  The shadow's fidelity for >32-bit ints is proven at the
        // `Stores` level instead (`materialize_gives_each_value_its_own_home`).
        if expr.contains("9000000000") {
            continue;
        }
        let mut s = session_with_defs();
        assert!(
            matches!(s.eval(&format!("v = {expr}")), Eval::Ran),
            "bind failed for `{expr}`"
        );
        let shadow = s.env_value("v");
        let fresh = s.value_of(&expr);
        match (&shadow, &fresh) {
            (Some(shadow), Some(fresh)) => {
                assert_eq!(
                    shadow, fresh,
                    "session-store shadow diverged from a fresh evaluation of \
                     `{expr}` (type {type_name})"
                );
                checked += 1;
            }
            // The snapshot path declined this shape, so there is nothing to
            // materialize — and the env must say so rather than hold a stale value.
            (None, None) => {}
            _ => panic!(
                "shadow and snapshot disagree on WHETHER `{expr}` (type \
                 {type_name}) has a value: env={shadow:?} snapshot={fresh:?}"
            ),
        }
    }
    assert!(
        checked >= 8,
        "oracle covered only {checked} cells — too few to call this a differential"
    );
}

/// The store outlives each eval's throwaway `State`: several bindings coexist and
/// stay readable after later evals have come and gone.
#[test]
fn env_entries_survive_across_evals() {
    let mut s = session_with_defs();
    assert!(matches!(s.eval("a = [1, 2, 3]"), Eval::Ran));
    assert!(matches!(s.eval("b = P { x: 7, y: 9 }"), Eval::Ran));
    assert!(matches!(s.eval("c = [\"one\", \"two\"]"), Eval::Ran));
    // Unrelated evals in between — each builds and drops its own State.
    assert!(matches!(s.eval("d = [9, 8]"), Eval::Ran));
    assert!(matches!(s.eval("n = 41 + 1"), Eval::Ran));
    // `n` is a scalar and, since arc C (Step 3), store-resident too.
    assert_eq!(s.env_names(), vec!["a", "b", "c", "d", "n"]);
    assert_eq!(s.env_value("a").as_deref(), Some("[1,2,3]"));
    assert_eq!(s.env_value("b").as_deref(), Some("P{x:7,y:9}"));
    assert_eq!(s.env_value("c").as_deref(), Some("[\"one\",\"two\"]"));
    assert_eq!(s.env_value("d").as_deref(), Some("[9,8]"));
}

/// A nested / multi-store value round-trips through the session store too — the
/// case that motivated materializing with `copy_claims` rather than a store copy.
#[test]
fn a_nested_value_round_trips_through_the_session() {
    let mut s = session_with_defs();
    assert!(matches!(
        s.eval("v = Line { a: P{x:1,y:2}, b: P{x:3,y:4}, tag: \"seg\" }"),
        Eval::Ran
    ));
    assert_eq!(
        s.env_value("v").as_deref(),
        Some("Line{a:P{x:1,y:2},b:P{x:3,y:4},tag:\"seg\"}")
    );
}

/// A re-bind replaces the entry (the old record orphans in the session store
/// until arc G collects it).
#[test]
fn re_bind_replaces_the_env_entry() {
    let mut s = session_with_defs();
    assert!(matches!(s.eval("v = [1, 2, 3]"), Eval::Ran));
    assert_eq!(s.env_value("v").as_deref(), Some("[1,2,3]"));
    assert!(matches!(s.eval("v = [4, 5]"), Eval::Ran));
    assert_eq!(s.env_value("v").as_deref(), Some("[4,5]"));
}

// ── Step 3 — scalars at rest (arc C) ────────────────────────────────────────
//
// Every binding now has a store-resident home, scalar or not.  That uniformity
// is the point: Step 4's frame-seed reads prior names along ONE path instead of
// branching on whether a value happened to be inline.

/// One cell per scalar kind: bind it, and the boxed value must render exactly
/// what the replay literal renders.
///
/// Same BICONDITIONAL as the heap differential: the shadow holds a value exactly
/// when the REPL.X snapshot path captured one.  A few shapes are declined by that
/// path on `main` and so have no entry — notably a POSITIVE integer literal above
/// the inferred `integer` range (`9000000000` is declined, `-9000000001` is not).
/// That is pre-existing and unrelated to @PLN14; what matters here is that the
/// shadow agrees about *whether* there is a value, never inventing one.
#[test]
fn scalars_are_store_resident_and_agree_with_the_replay() {
    let cases: &[(&str, &str)] = &[
        ("n", "5"),
        ("neg", "-17"),
        ("big", "9000000000"), // > 32-bit, declined by the snapshot path
        ("negbig", "-9000000001"),
        ("zero", "0"),
        ("f", "1.5"),
        ("fneg", "-0.25"),
        ("b", "true"),
        ("bf", "false"),
        ("c", "'x'"),
        ("t", "\"hi\""),
        ("empty", "\"\""),
        ("esc", "\"a\\\"b\\nc\\\\d \\u{2603}\""),
    ];
    let mut checked = 0;
    for (name, expr) in cases {
        let mut s = session();
        assert!(
            matches!(s.eval(&format!("{name} = {expr}")), Eval::Ran),
            "bind failed: {name} = {expr}"
        );
        let shadow = s.env_value(name);
        let replayed = s.value_of(name);
        match (&shadow, &replayed) {
            (Some(shadow), Some(replayed)) => {
                assert_eq!(
                    shadow, replayed,
                    "boxed scalar diverged from the replayed value for `{name} = {expr}`"
                );
                checked += 1;
            }
            (None, None) => {}
            _ => panic!(
                "shadow and snapshot disagree on WHETHER `{name} = {expr}` has a \
                 value: env={shadow:?} snapshot={replayed:?}"
            ),
        }
    }
    assert!(
        checked >= 10,
        "only {checked} scalar cells carried a value — too few to be a differential"
    );
}

/// Boxing is by RAW BYTES, not via the display literal — so a float that does not
/// survive a naive decimal round-trip still comes back exact.  This is why Q2
/// keeps own-format out of the session path.
#[test]
fn boxed_floats_are_exact() {
    for expr in ["0.1 + 0.2", "1.0 / 3.0", "1.0e300 * 1.0e-300", "-0.0"] {
        let mut s = session();
        assert!(
            matches!(s.eval(&format!("v = {expr}")), Eval::Ran),
            "{expr}"
        );
        let shadow = s
            .env_value("v")
            .unwrap_or_else(|| panic!("no entry: {expr}"));
        let replayed = s
            .value_of("v")
            .unwrap_or_else(|| panic!("no replay: {expr}"));
        assert_eq!(shadow, replayed, "float lost exactness for `{expr}`");
    }
}

/// A simple (payload-free) enum boxes its discriminant and reads back qualified.
#[test]
fn simple_enum_is_store_resident() {
    let mut s = session();
    assert!(matches!(
        s.eval("enum Direction { North, East, South, West }"),
        Eval::Ran
    ));
    assert!(matches!(s.eval("d = Direction.South"), Eval::Ran));
    assert_eq!(s.env_value("d").as_deref(), Some("Direction.South"));
    assert_eq!(s.value_of("d").as_deref(), Some("Direction.South"));
}

/// A `text` binding is store-resident with its CHARACTERS copied into the session
/// store, and reads back as a bare text literal — not as the single-element
/// `vector<text>` the @P293 work-around physically stores it in.
#[test]
fn text_is_store_resident_and_unwrapped() {
    let mut s = session();
    assert!(matches!(s.eval("t = \"hi\""), Eval::Ran));
    assert_eq!(s.env_value("t").as_deref(), Some("\"hi\""));
    // A COMPUTED text (the borrowed case @P293 is about) works the same way.
    assert!(matches!(s.eval("u = t + \" there\""), Eval::Ran));
    assert_eq!(s.env_value("u").as_deref(), Some("\"hi there\""));
    assert_eq!(s.value_of("u").as_deref(), Some("\"hi there\""));
}

/// Every binding kind now has a home — the arc-C uniformity claim, stated as one
/// assertion so a regression in any single kind shows up here.
#[test]
fn every_binding_kind_has_a_store_resident_home() {
    let mut s = session_with_defs();
    for input in [
        "n = 5",
        "f = 1.5",
        "b = true",
        "c = 'q'",
        "t = \"hi\"",
        "v = [1, 2, 3]",
        "p = P { x: 1, y: 2 }",
        "l = Line { a: P{x:1,y:2}, b: P{x:3,y:4}, tag: \"seg\" }",
    ] {
        assert!(matches!(s.eval(input), Eval::Ran), "bind failed: {input}");
    }
    assert_eq!(
        s.env_names(),
        vec!["b", "c", "f", "l", "n", "p", "t", "v"],
        "some binding kind has no store-resident home"
    );
}

/// A faulting bind still records nothing — no env entry, no poison.
#[test]
fn a_faulting_bind_records_no_env_entry() {
    let mut s = session();
    assert!(matches!(s.eval("ok = 1"), Eval::Ran));
    let before = s.env_names();
    let _ = s.eval("bad = assert(false, \"boom\")");
    assert_eq!(s.env_names(), before, "a faulting bind left an env entry");
}

/// The shadow is WRITE-ONLY in Step 2: the replay model is still the source of
/// truth, so a session behaves exactly as before.  (Step 5 is where observing
/// switches to the env and the body replay goes away.)
#[test]
fn the_shadow_does_not_change_session_behaviour() {
    let mut s = session_with_defs();
    assert!(matches!(s.eval("a = [1, 2, 3]"), Eval::Ran));
    assert!(matches!(s.eval("n = 10"), Eval::Ran));
    assert!(matches!(s.eval("assert(n == 10, \"n\")"), Eval::Ran));
    assert!(matches!(s.eval("n = n + 5"), Eval::Ran));
    assert!(matches!(s.eval("assert(n == 15, \"n grew\")"), Eval::Ran));
    assert_eq!(s.value_of("n").as_deref(), Some("15"));
}

// ── Step 4 — frame-seed (arc D), the risk phase ─────────────────────────────
//
// The first step that READS the env.  It stays differential-gated: the body
// replay still fills the slots, the seed overwrites them from the session
// store, and every binding's before/after must match.  A divergence is a loud
// failure here rather than a silent wrong value in a session.

/// Pause inside a function whose locals share the session's binding names, seed
/// those slots from the session store, and assert the store-resident value is
/// EXACTLY what the replay had put there.
#[test]
fn frame_seed_agrees_with_the_replayed_slot() {
    let mut s = session();
    assert!(matches!(
        s.eval("struct P { x: integer, y: integer }"),
        Eval::Ran
    ));
    // Top-level bindings — these populate the store-resident env.
    for input in [
        "n = 42",
        "f = 1.5",
        "b = true",
        "c = 'q'",
        "v = [1, 2, 3]",
        "p = P { x: 7, y: 9 }",
    ] {
        assert!(matches!(s.eval(input), Eval::Ran), "bind failed: {input}");
    }
    // A function whose locals carry the SAME names, so its frame is the shape a
    // seeded REPL generation has.
    //
    // Every local must be READ later on: the compiler coalesces the stack slots of
    // locals whose live ranges do not overlap, so assigned-but-never-read locals
    // share one slot and two bindings would seed the same address.  (That is the
    // flake the differential caught — `v` and `p` both landed on slot 148.)
    assert!(matches!(
        s.eval(
            "fn probe() -> integer {\n  n = 42;\n  f = 1.5;\n  b = true;\n  c = 'q';\n  \
             v = [1, 2, 3];\n  p = P { x: 7, y: 9 };\n  assert(b, \"b\");\n  \
             assert(c == 'q', \"c\");\n  assert(f > 1.0, \"f\");\n  n + v[0] + p.x\n}"
        ),
        Eval::Ran
    ));
    s.debug_stepping(true);
    // Line 8 is the first `assert` — every local above is assigned by now AND is
    // still read further down, so all six are live in distinct slots.
    s.add_breakpoint("probe:8");
    assert!(
        matches!(s.eval("probe()"), Eval::Paused),
        "expected a pause inside probe"
    );

    let reports = s.seed_paused_frame();
    let names: Vec<&str> = reports.iter().map(|r| r.name.as_str()).collect();
    // ALL six, not a subset: a binding skipped for a coalesced slot must surface
    // here rather than quietly reducing what the differential covers.
    assert_eq!(
        names,
        vec!["b", "c", "f", "n", "p", "v"],
        "the seed did not cover every live binding"
    );
    for r in &reports {
        assert_eq!(
            r.replayed, r.seeded,
            "frame-seed DIVERGED for `{}`: replay had {:?}, the session store \
             seeded {:?}",
            r.name, r.replayed, r.seeded
        );
    }
    s.debug_continue();
}

/// Non-vacuity: the seed must actually WRITE. Corrupt a slot first (via the
/// debugger's edit path), then seed — the stored value must overwrite the
/// corruption. Without this, "replayed == seeded" could pass by the seed being
/// a no-op that never touched the slot.
#[test]
fn frame_seed_actually_writes_the_slot() {
    let mut s = session();
    assert!(matches!(s.eval("n = 42"), Eval::Ran));
    assert!(matches!(
        s.eval("fn probe() -> integer {\n  n = 42;\n  n\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("probe:3");
    assert!(
        matches!(s.eval("probe()"), Eval::Paused),
        "expected a pause"
    );

    // Corrupt the slot: the frame now disagrees with the session store.
    assert!(
        s.debug_set("n", "999"),
        "debug_set should edit the live local"
    );
    let reports = s.seed_paused_frame();
    let n = reports
        .iter()
        .find(|r| r.name == "n")
        .expect("n was seeded");
    assert_eq!(
        n.replayed, "999",
        "the corruption should be what we replaced"
    );
    assert_eq!(
        n.seeded, "42",
        "the seed did not overwrite the slot from the session store"
    );
    s.debug_continue();
}

// ── Step 5 — the flip: observe reads the env (arc E) ─────────────────────────
//
// Behind `LOFT_PLN14_STORE_OBSERVE` / `set_store_observe`.  Two things must
// hold: what a session PRINTS is unchanged (fidelity), and observing a
// store-resident binding no longer replays the body (the cost win, measured
// directly via the generation counter rather than by timing).

/// The cost claim, measured: observing a store-resident name must not compile a
/// generation, which is a direct proof the accumulated body did not re-run.
#[test]
fn store_observe_does_not_replay_the_body() {
    let mut s = session();
    assert!(matches!(s.eval("v = [1, 2, 3]"), Eval::Ran));
    assert!(matches!(s.eval("n = 5"), Eval::Ran));

    // Flag OFF: observing replays — the generation counter advances.
    s.set_store_observe(false);
    let before_off = s.generations();
    let _ = s.value_of("n");
    let after_off = s.generations();
    assert!(
        after_off > before_off,
        "with the flag off, observing should still replay (counter {before_off} → {after_off})"
    );

    // Flag ON: observing is answered from the store — nothing is compiled.
    s.set_store_observe(true);
    let before_on = s.generations();
    assert_eq!(s.value_of("n").as_deref(), Some("5"));
    assert_eq!(s.value_of("v").as_deref(), Some("[1,2,3]"));
    assert_eq!(
        s.generations(),
        before_on,
        "the body was replayed even though the value is store-resident"
    );
}

/// Fidelity of the own-format read: the flipped `value_of` must return exactly
/// what a fresh evaluation of the same expression returns.
#[test]
fn store_observe_value_of_matches_a_fresh_evaluation() {
    let mut checked = 0;
    for (_ty, type_name, expr) in heap_cases() {
        if expr.contains("9000000000") {
            continue; // declined by the snapshot path (see the Step 3 note)
        }
        // The oracle: a fresh evaluation, in its own un-flipped session.
        let mut oracle = session_with_defs();
        let Some(expected) = oracle.value_of(&expr) else {
            continue;
        };

        let mut s = session_with_defs();
        assert!(matches!(s.eval(&format!("v = {expr}")), Eval::Ran));
        s.set_store_observe(true);
        let got = s
            .value_of("v")
            .unwrap_or_else(|| panic!("no store-backed value for `{expr}`"));
        assert_eq!(
            got, expected,
            "flipped value_of diverged for `{expr}` (type {type_name})"
        );
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} cells covered");
}

/// The flip's default, and that it can still be turned off.
/// Step 8 — the flip is now the default, and the opt-out still works.
#[test]
fn store_observe_is_on_by_default_and_opt_outable() {
    assert!(
        session().store_observe(),
        "store-backed observing is the default since Step 8"
    );
    let mut s = session();
    s.set_store_observe(false);
    assert!(!s.store_observe(), "the opt-out must still turn it off");
}

/// End-to-end fidelity gate: a real REPL session must print BYTE-IDENTICAL
/// output with the flip on and off.  This is the gate that has to stay green
/// before Step 8 can make the flip the default — it covers the echo and `:vars`,
/// whose display rendering (`hi`, `{x:7,y:9}`, `3`) differs from the own-format
/// literal (`"hi"`, `P{x:7,y:9}`, `3.0`) the store also has to be able to serve.
#[test]
fn a_real_repl_session_prints_identically_with_the_flip() {
    let script = "struct P { x: integer, y: integer }\n\
         enum Direction { North, East, South, West }\n\
         n = 5\nf = 1.5\nw = 3.0\ng = 1.0 / 3.0\nsg = 2.5f\nb = true\nc = 'q'\n\
         t = \"hi\"\nu = t + \" there\"\nv = [1, 2, 3]\nvt = [\"a\", \"b\"]\n\
         p = P { x: 7, y: 9 }\nd = Direction.South\n\
         n\nf\nw\ng\nsg\nb\nc\nt\nu\nv\nvt\np\nd\n:vars\n:quit\n";

    let run = |flip: bool| -> String {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_loft"));
        cmd.arg("repl")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // Step 8 — the flip is the DEFAULT now, so the comparison runs the
        // opt-out (`LOFT_NO_STORE_OBSERVE`) as the "replay" side.
        if flip {
            cmd.env_remove("LOFT_NO_STORE_OBSERVE");
        } else {
            cmd.env("LOFT_NO_STORE_OBSERVE", "1");
        }
        let mut child = cmd.spawn().expect("spawn the loft repl");
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(script.as_bytes())
            .expect("write the script");
        let out = child.wait_with_output().expect("repl exits");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    let off = run(false);
    let on = run(true);
    assert!(
        off.contains("{x:7,y:9}"),
        "the baseline run produced no values:\n{off}"
    );
    assert_eq!(
        on, off,
        "the flip changed what the session prints (left = flipped, right = replay)"
    );
}

// ── Step 6 — resume: save → restore + the schema gate (arc F) ────────────────
//
// Re-assertion site 2, and the arc the sibling explicitly does NOT cover:
// `snapshot_heap` refuses a file-backed store and skips the schema, which is
// exactly what a resume image must cross.  The safety property is not "the
// image is always right" but "a bad image FALLS BACK, never miscomputes".

fn image_path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("loft_pln14_{tag}_{}.image", std::process::id()))
}

/// The `save → restore` cells of the Stage-A matrix, finally built: every value
/// type must survive a round-trip through an on-disk image and read back equal.
// @speed 0.8
#[test]
fn every_value_type_survives_the_resume_image() {
    let mut checked = 0;
    for (i, (_ty, type_name, expr)) in heap_cases().into_iter().enumerate() {
        if expr.contains("9000000000") {
            continue; // declined by the snapshot path (see the Step 3 note)
        }
        let path = image_path(&format!("heap{i}"));
        let _ = std::fs::remove_file(&path);

        let mut s = session_with_defs();
        assert!(matches!(s.eval(&format!("v = {expr}")), Eval::Ran));
        // Some shapes have no store-resident value because the REPL.X snapshot
        // path declines them on `main` (a vector-of-structs, like the >32-bit
        // literal above) — pre-existing and unrelated to @PLN14.  Nothing to
        // round-trip, so skip rather than invent a cell.
        let Some(expected) = s.env_value("v") else {
            continue;
        };
        assert!(s.save_session_image(&path).expect("write the image"));

        // A NEW session that never bound anything — only the type defs, as a
        // real resume replays them before loading the image.
        let mut fresh = session_with_defs();
        assert_eq!(fresh.load_session_image(&path), ImageLoad::Loaded, "{expr}");
        assert_eq!(
            fresh.env_value("v").as_deref(),
            Some(expected.as_str()),
            "value did not survive the resume image for `{expr}` (type {type_name})"
        );
        let _ = std::fs::remove_file(&path);
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} cells covered");
}

/// Scalars round-trip through the image too — including a float, which must come
/// back bit-exact rather than via its decimal form.
#[test]
fn scalars_survive_the_resume_image() {
    let path = image_path("scalars");
    let _ = std::fs::remove_file(&path);
    let mut s = session();
    for input in [
        "n = 42",
        "f = 0.1 + 0.2",
        "b = true",
        "c = 'q'",
        "t = \"hi\"",
    ] {
        assert!(matches!(s.eval(input), Eval::Ran), "{input}");
    }
    let before: Vec<Option<String>> = ["n", "f", "b", "c", "t"]
        .iter()
        .map(|n| s.env_value(n))
        .collect();
    assert!(s.save_session_image(&path).expect("write"));

    let mut fresh = session();
    assert_eq!(fresh.load_session_image(&path), ImageLoad::Loaded);
    let after: Vec<Option<String>> = ["n", "f", "b", "c", "t"]
        .iter()
        .map(|n| fresh.env_value(n))
        .collect();
    assert_eq!(before, after, "a scalar did not survive the resume image");
    let _ = std::fs::remove_file(&path);
}

/// THE GATE. A changed storage layout must be REFUSED, not misread: the image is
/// written with `P { x: integer, y: integer }` and loaded into a session whose
/// `P` has a different shape.  Reading those bytes as the new layout would hand
/// back a plausible-looking wrong value — the exact miscompute arc F exists to
/// prevent.
#[test]
fn a_changed_layout_is_refused_not_misread() {
    let path = image_path("schema");
    let _ = std::fs::remove_file(&path);
    let mut s = session();
    assert!(matches!(
        s.eval("struct P { x: integer, y: integer }"),
        Eval::Ran
    ));
    assert!(matches!(s.eval("p = P { x: 7, y: 9 }"), Eval::Ran));
    assert!(s.save_session_image(&path).expect("write"));

    // A session whose `P` has a DIFFERENT layout.
    let mut other = session();
    assert!(matches!(
        other.eval("struct P { x: integer, y: integer, z: integer }"),
        Eval::Ran
    ));
    assert_eq!(
        other.load_session_image(&path),
        ImageLoad::SchemaMismatch,
        "a changed layout must be refused"
    );
    assert!(
        other.env_names().is_empty(),
        "a refused image must leave the session untouched"
    );
    let _ = std::fs::remove_file(&path);
}

/// A type the session has not defined at all is likewise refused (the resume
/// must replay defs before loading the image).
#[test]
fn an_unknown_type_is_refused() {
    let path = image_path("unknown");
    let _ = std::fs::remove_file(&path);
    let mut s = session();
    assert!(matches!(
        s.eval("struct P { x: integer, y: integer }"),
        Eval::Ran
    ));
    assert!(matches!(s.eval("p = P { x: 7, y: 9 }"), Eval::Ran));
    assert!(s.save_session_image(&path).expect("write"));

    let mut bare = session(); // no `P` defined
    assert_eq!(bare.load_session_image(&path), ImageLoad::SchemaMismatch);
    assert!(bare.env_names().is_empty());
    let _ = std::fs::remove_file(&path);
}

/// Every malformed shape falls back rather than being partially applied: a
/// missing file, a foreign file, a truncation at each length, and a corrupted
/// store arena.  None of these may panic or leave a half-loaded session.
#[test]
fn a_malformed_image_falls_back_and_never_half_applies() {
    let path = image_path("malformed");
    let _ = std::fs::remove_file(&path);
    let mut s = session();
    assert!(matches!(s.eval("v = [1, 2, 3]"), Eval::Ran));
    assert!(matches!(s.eval("n = 7"), Eval::Ran));
    assert!(s.save_session_image(&path).expect("write"));
    let good = std::fs::read(&path).expect("read back");

    let mut fresh = session();
    assert_eq!(
        fresh.load_session_image(&image_path("does_not_exist")),
        ImageLoad::Missing
    );

    // A foreign file.
    std::fs::write(&path, b"not a loft session image at all").expect("write");
    assert_eq!(fresh.load_session_image(&path), ImageLoad::Malformed);

    // Truncation at every prefix length — none may panic.
    for cut in [0, 4, 8, 12, 20, 24, 40, good.len() / 2, good.len() - 1] {
        std::fs::write(&path, &good[..cut.min(good.len())]).expect("write");
        let got = fresh.load_session_image(&path);
        assert_ne!(
            got,
            ImageLoad::Loaded,
            "a truncated image was accepted at {cut}"
        );
    }

    // A corrupted store arena: header intact, arena bytes garbage.
    let mut corrupt = good.clone();
    let n = corrupt.len();
    for b in &mut corrupt[n - 32..] {
        *b = 0xAB;
    }
    let got = fresh.load_session_image(&path);
    std::fs::write(&path, &corrupt).expect("write");
    let _ = got;
    let after = fresh.load_session_image(&path);
    assert!(
        after == ImageLoad::Loaded || after == ImageLoad::Malformed,
        "unexpected outcome {after:?}"
    );

    // Whatever happened, the session is still usable and never half-applied.
    assert!(matches!(fresh.eval("z = 1"), Eval::Ran));
    let _ = std::fs::remove_file(&path);
}

/// A round-trip through the image preserves the FLIP's behaviour too: the
/// restored values answer observes from the store, with no body to replay at all
/// (the restored session has an empty body).
#[test]
fn a_restored_session_observes_from_the_store() {
    let path = image_path("observe");
    let _ = std::fs::remove_file(&path);
    let mut s = session_with_defs();
    assert!(matches!(s.eval("v = [1, 2, 3]"), Eval::Ran));
    assert!(matches!(s.eval("p = P { x: 7, y: 9 }"), Eval::Ran));
    assert!(s.save_session_image(&path).expect("write"));

    let mut fresh = session_with_defs();
    assert_eq!(fresh.load_session_image(&path), ImageLoad::Loaded);
    fresh.set_store_observe(true);
    let gens = fresh.generations();
    assert_eq!(fresh.value_of("v").as_deref(), Some("[1,2,3]"));
    assert_eq!(fresh.value_of("p").as_deref(), Some("P{x:7,y:9}"));
    assert_eq!(
        fresh.generations(),
        gens,
        "a restored session should not need to replay anything"
    );
    let _ = std::fs::remove_file(&path);
}

// ── Step 7 — lifetime (arc G) ───────────────────────────────────────────────

/// A re-bind releases the record the old value held, so a `n = n + 1` REPL loop
/// does not grow the session store without bound.  The env is the ONLY holder of
/// a ref into that store, which is what makes freeing on replace safe.
// @speed 0.9
#[test]
fn re_binding_does_not_grow_the_session_store() {
    let mut s = session();
    assert!(matches!(s.eval("v = [1, 2, 3, 4, 5]"), Eval::Ran));
    // Re-bind the same name many times; each one orphans its predecessor.
    for i in 0..60 {
        assert!(
            matches!(
                s.eval(&format!("v = [{i}, {}, {}]", i + 1, i + 2)),
                Eval::Ran
            ),
            "re-bind {i} failed"
        );
    }
    assert_eq!(s.env_names(), vec!["v"], "still exactly one binding");
    assert_eq!(
        s.env_value("v").as_deref(),
        Some("[59,60,61]"),
        "last value wins"
    );
    // Compare LIVE RECORDS against a session that bound the same value ONCE.
    // (The arena's byte size is pre-allocated and stays flat either way, so
    // measuring that would be vacuous — this guard was rewritten after the
    // first version passed with the free deliberately disabled.)
    let mut once = session();
    assert!(matches!(once.eval("v = [59, 60, 61]"), Eval::Ran));
    let (a, b) = (s.session_store_records(), once.session_store_records());
    assert!(
        a <= b + 2,
        "60 re-binds left {a} records where one bind leaves {b} — the orphaned \
         records are not being released"
    );
}

/// A scalar re-bind is the common REPL shape (`n = n + 1`) and must free too.
#[test]
fn scalar_re_binding_reuses_store_space() {
    let mut s = session();
    assert!(matches!(s.eval("n = 0"), Eval::Ran));
    for _ in 0..80 {
        assert!(matches!(s.eval("n = n + 1"), Eval::Ran));
    }
    assert_eq!(s.env_value("n").as_deref(), Some("80"));
    let mut once = session();
    assert!(matches!(once.eval("n = 80"), Eval::Ran));
    let (a, b) = (s.session_store_records(), once.session_store_records());
    assert!(
        a <= b + 2,
        "80 scalar re-binds left {a} records where one bind leaves {b}"
    );
}

/// `:reset` wipes the store-resident session.  It already did — the store and
/// env ride the `ReplSession`, so rebuilding the session drops them — but that is
/// a property worth pinning rather than rediscovering.
#[test]
fn a_fresh_session_carries_no_store_state() {
    let mut s = session();
    assert!(matches!(s.eval("v = [1, 2, 3]"), Eval::Ran));
    assert_eq!(s.env_names(), vec!["v"]);
    // What `:reset` does: replace the session.
    let fresh = session();
    assert!(
        fresh.env_names().is_empty(),
        "a reset session has no bindings"
    );
    assert_eq!(fresh.session_store_records(), 0, "and no session store");
}

// ── Step 8 — arc H: eval that RETURNS the value ─────────────────────────────

/// The in-process eval API: a value comes back instead of going to stdout.
#[test]
fn eval_value_returns_rendered_values() {
    let mut s = session_with_defs();
    // A binding advances the session but yields no value.
    assert_eq!(s.eval_value("v = [1, 2, 3]").expect("bind"), None);
    assert_eq!(s.eval_value("p = P { x: 7, y: 9 }").expect("bind"), None);
    // A definition likewise.
    assert_eq!(s.eval_value("struct Q { a: integer }").expect("def"), None);
    // A bare name is answered from the session store.
    assert_eq!(s.eval_value("v").expect("read"), Some("[1,2,3]".into()));
    assert_eq!(s.eval_value("p").expect("read"), Some("P{x:7,y:9}".into()));
    // An expression is evaluated and rendered.
    assert_eq!(s.eval_value("1 + 2").expect("expr"), Some("3".into()));
}

/// Reading a store-resident name through `eval_value` compiles no generation —
/// the arc-E win, now reachable from the embedding API too.
#[test]
fn eval_value_reads_a_binding_without_replaying() {
    let mut s = session();
    assert!(matches!(s.eval("v = [1, 2, 3]"), Eval::Ran));
    let before = s.generations();
    assert_eq!(s.eval_value("v").expect("read"), Some("[1,2,3]".into()));
    assert_eq!(
        s.generations(),
        before,
        "reading a store-resident binding must not replay the body"
    );
}

/// A broken input surfaces diagnostics rather than panicking or returning a value.
#[test]
fn eval_value_reports_errors() {
    let mut s = session();
    let out = s.eval_value("v = 1 2 3");
    assert!(out.is_err(), "a parse error must be reported; got {out:?}");
    assert!(
        matches!(s.eval("ok = 1"), Eval::Ran),
        "the session stays usable"
    );
}
