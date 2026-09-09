// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Validation tests for the variable introspection framework
//! ([`State::iter_frame_variables`] and [`State::dump_frame_variables`]).
//!
//! Each test runs a small known-good loft program through `execute_log` and
//! asserts that the framework correctly identifies live variables and reads
//! their values from the runtime stack.

extern crate loft;

use loft::compile::byte_code;
use loft::data::Data;
use loft::log_config::LogConfig;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;

/// Compile `script`, run via `execute_log`, and after each opcode call
/// `inspect(state, data)` so the test can assert intermediate state.
///
/// Returns the captured trace string.
fn build(script: &str) -> (State, Data) {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(script, "frame_vars", false);
    if !p.diagnostics.is_empty() {
        panic!("parse errors: {:?}", p.diagnostics.lines());
    }
    scopes::check(&mut p.data, &mut p.database);
    let mut data = p.data;
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut data);
    (state, data)
}

// ── Test 1: Single integer variable ─────────────────────────────────────────

#[test]
fn integer_variable_layout() {
    // After compile, the framework can list slot-assigned variables for
    // n_test.  We don't need execute output to verify the slot table.
    let (_state, data) = build(
        "fn test() {
    x = 42;
    y = 100;
    z = x + y;
    assert(z == 142);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    let vars = &data.def(fn_d_nr).variables;
    // x, y, z all exist as variables.
    let names: Vec<String> = (0..vars.count())
        .map(|i| vars.name(i).to_string())
        .collect();
    assert!(names.iter().any(|n| n == "x"), "no var x in {names:?}");
    assert!(names.iter().any(|n| n == "y"), "no var y in {names:?}");
    assert!(names.iter().any(|n| n == "z"), "no var z in {names:?}");
    // All have integer type.
    for v_nr in 0..vars.count() {
        let n = vars.name(v_nr);
        if n == "x" || n == "y" || n == "z" {
            assert!(
                matches!(vars.tp(v_nr), loft::data::Type::Integer(_)),
                "{n} should be Integer, got {:?}",
                vars.tp(v_nr)
            );
        }
    }
}

// ── Test 2: Iterator yields variables for entry function ────────────────────

#[test]
fn iter_yields_function_variables() {
    let (mut state, data) = build(
        "fn test() {
    a = 10;
    b = 20;
    assert(a + b == 30);
}",
    );
    // Set code_pos to the start of n_test so iter_frame_variables can locate
    // the function.  Don't actually execute — just check the data shape.
    let fn_d_nr = data.def_nr("n_test");
    state.code_pos = data.def(fn_d_nr).code_position;
    state.stack_pos = 4; // entry function start
    let frame_vars = state.iter_frame_variables(&data);
    // At least a and b should appear.
    let names: Vec<&str> = frame_vars.iter().map(|v| v.name.as_str()).collect();
    assert!(
        names.contains(&"a"),
        "iter did not yield 'a': got {names:?}"
    );
    assert!(
        names.contains(&"b"),
        "iter did not yield 'b': got {names:?}"
    );
}

// ── Test 3: Liveness — slot-coalesced variables are marked correctly ───────

#[test]
fn liveness_marks_dead_variables() {
    // This script reuses slots: `x` is dead after `z = x + y`, so its slot
    // is coalesced with another variable.
    let (mut state, data) = build(
        "fn test() {
    x = 42;
    y = 100;
    z = x + y;
    assert(z == 142);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    // Position at the very start of the function: nothing referenced yet,
    // all locals should be marked NOT live.
    state.code_pos = data.def(fn_d_nr).code_position;
    state.stack_pos = 4;
    let vars = state.iter_frame_variables(&data);
    for v in &vars {
        if !v.is_argument {
            assert!(
                !v.live,
                "var '{}' marked live before any reference (bc_first={}, code_pos={})",
                v.name, v.bc_first, state.code_pos
            );
        }
    }
}

// ── Test 4: Bytecode-position liveness range is populated ───────────────────

#[test]
fn liveness_range_populated() {
    let (mut state, data) = build(
        "fn test() {
    x = 7;
    assert(x == 7);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    // Pick a code position inside the function body.
    state.code_pos = data.def(fn_d_nr).code_position + 10;
    state.stack_pos = 4;
    let vars = state.iter_frame_variables(&data);
    let x = vars
        .iter()
        .find(|v| v.name == "x")
        .expect("variable x missing");
    assert!(x.bc_first != u32::MAX, "x has no bytecode reference range");
    assert!(
        x.bc_last >= x.bc_first,
        "x bc_last={} < bc_first={}",
        x.bc_last,
        x.bc_first
    );
    // The range must lie within the function bytecode.
    let fn_start = data.def(fn_d_nr).code_position;
    let fn_end = fn_start + data.def(fn_d_nr).code_length;
    assert!(
        x.bc_first >= fn_start && x.bc_last < fn_end,
        "x range [{}, {}] outside function [{}, {})",
        x.bc_first,
        x.bc_last,
        fn_start,
        fn_end
    );
}

// ── Test 5: Iterator is read-only — does not mutate stack_pos ──────────────

#[test]
fn iter_does_not_mutate_state() {
    let (mut state, data) = build(
        "fn test() {
    n = 5;
    assert(n == 5);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    state.code_pos = data.def(fn_d_nr).code_position;
    let stack_before = state.stack_pos;
    let code_before = state.code_pos;
    let _ = state.iter_frame_variables(&data);
    let _ = state.iter_frame_variables(&data);
    assert_eq!(state.stack_pos, stack_before, "stack_pos changed");
    assert_eq!(state.code_pos, code_before, "code_pos changed");
}

// ── Test 6: dump_frame_variables produces expected format ──────────────────

#[test]
fn dump_format_smoke_test() {
    let (mut state, data) = build(
        "fn test() {
    n = 99;
    assert(n == 99);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    state.code_pos = data.def(fn_d_nr).code_position;
    state.stack_pos = 4;
    let mut buf = Vec::<u8>::new();
    state.dump_frame_variables(&mut buf, &data).unwrap();
    let out = String::from_utf8(buf).unwrap();
    assert!(
        out.starts_with("[VARS]"),
        "dump should start with [VARS]: {out}"
    );
    assert!(
        out.contains("fn=n_test"),
        "dump should name function: {out}"
    );
}

// ── Test 7: Run a real script through execute_log to ensure no crashes ────

#[test]
fn execute_log_with_dump_does_not_crash() {
    let (mut state, data) = build(
        "fn test() {
    a = 1;
    b = 2;
    assert(a + b == 3);
}",
    );
    let mut config = LogConfig::full();
    config.dump_vars = true;
    let mut buf = Vec::<u8>::new();
    state.execute_log(&mut buf, "test", &config, &data).unwrap();
    let out = String::from_utf8(buf).unwrap();
    assert!(out.contains("[VARS]"), "no [VARS] lines in trace");
    assert!(out.contains("fn=n_test"), "no n_test frame: {out}");
    // The integer values 1, 2, 3 should appear in the trace.
    assert!(out.contains("i32"), "no i32 type in dump");
}

// ── Test 8a: Verify the framework agrees with the codegen vars map ─────────

#[test]
fn iter_var_nr_matches_codegen() {
    // The codegen records (bytecode_pos, var_nr) entries in State.vars when
    // it emits a variable-accessing opcode.  The framework's iter_frame_variables
    // looks up vars by var_nr.  These two views must agree on slot positions.
    let (mut state, data) = build(
        "fn test() {
    a = 1;
    b = 2;
    c = a + b;
    assert(c == 3);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    state.code_pos = data.def(fn_d_nr).code_position;
    let frame_vars = state.iter_frame_variables(&data);
    // For each variable in the codegen vars map (within this function), look
    // up its name and slot via the framework — they must match.
    let fn_start = data.def(fn_d_nr).code_position;
    let fn_end = fn_start + data.def(fn_d_nr).code_length;
    let func_vars = &data.def(fn_d_nr).variables;
    for (&bc, &v_nr) in &state.vars {
        if bc < fn_start || bc >= fn_end {
            continue;
        }
        let codegen_name = func_vars.name(v_nr);
        let codegen_slot = func_vars.stack(v_nr);
        let frame_var = frame_vars.iter().find(|fv| fv.var_nr == v_nr);
        match frame_var {
            Some(fv) => {
                assert_eq!(
                    fv.name, codegen_name,
                    "var_nr={v_nr} name mismatch: frame={} codegen={codegen_name}",
                    fv.name
                );
                assert_eq!(
                    fv.slot, codegen_slot,
                    "var_nr={v_nr} ({codegen_name}) slot mismatch: frame={} codegen={codegen_slot}",
                    fv.slot
                );
            }
            None => {
                // var_nr might be filtered out (slot == u16::MAX), but if the
                // codegen referenced it, slot should be assigned.
                if codegen_slot != u16::MAX {
                    panic!(
                        "var_nr={v_nr} ({codegen_name}) slot={codegen_slot} \
                         missing from iter_frame_variables (bc={bc})"
                    );
                }
            }
        }
    }
}

// ── Test 8b: same validation on the file_content reproducer ────────────────

#[test]
fn iter_var_nr_matches_codegen_file_content() {
    let (mut state, data) = build(
        "fn test() {
    f = file(\"/nonexistent.txt\");
    t = f.content();
    assert(t == \"\");
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    state.code_pos = data.def(fn_d_nr).code_position;
    let frame_vars = state.iter_frame_variables(&data);
    let fn_start = data.def(fn_d_nr).code_position;
    let fn_end = fn_start + data.def(fn_d_nr).code_length;
    let func_vars = &data.def(fn_d_nr).variables;
    let mut errors = Vec::new();
    for (&bc, &v_nr) in &state.vars {
        if bc < fn_start || bc >= fn_end {
            continue;
        }
        let codegen_name = func_vars.name(v_nr).to_string();
        let codegen_slot = func_vars.stack(v_nr);
        if let Some(fv) = frame_vars.iter().find(|fv| fv.var_nr == v_nr)
            && fv.slot != codegen_slot
        {
            errors.push(format!(
                "var_nr={v_nr} ({codegen_name}) bc={bc}: frame slot={} != codegen slot={codegen_slot}",
                fv.slot
            ));
        }
    }
    assert!(
        errors.is_empty(),
        "{} mismatches:\n{}",
        errors.len(),
        errors.join("\n")
    );
}

// ── Test 9: Argument size — text args use Str (16B), locals use String (24B)

#[test]
fn arg_text_uses_str_layout() {
    let (mut state, data) = build(
        "fn helper(s: text) -> integer {
    s.len()
}
fn test() {
    n = helper(\"hello\");
    assert(n == 5);
}",
    );
    let helper_d_nr = data.def_nr("n_helper");
    state.code_pos = data.def(helper_d_nr).code_position;
    state.stack_pos = 4 + 16; // return addr + 16-byte Str arg
    let vars = state.iter_frame_variables(&data);
    let s = vars.iter().find(|v| v.name == "s").expect("missing arg s");
    assert!(s.is_argument, "s should be an argument");
    assert_eq!(s.size, 16, "arg text should use 16-byte Str layout");
}

// ── @PLN54 S3 — reserve-poison positive control ─────────────────────────────

/// Positive control for the stack half of `LOFT_POISON` (@PLN54 S3): prove the
/// detector CAN fire, so a green `LOFT_POISON=1` suite is non-vacuous.
///
/// `reserve_frame` grows the stack into a freshly-reserved, above-TOS region and
/// — under `LOFT_POISON=1` — fills it with the `0xDEADBEEF` sentinel (design:
/// `plans/54-sanitizer-coverage-expansion/STACK_POISON_DESIGN.md`).  So a read of
/// a slot the frame never wrote returns the sentinel; read as a `DbRef` its
/// `store_nr` is `0xBEEF` (wildly out of range → the store-index guard fires), a
/// dangling stack read made loud instead of silent stale data.
///
/// Inert without the flag (poison is a process-global cached env read), so this
/// runs meaningfully in the nightly `poison` CI job and is skipped in a normal
/// run.  Profile-independent (asserts the sentinel bytes, not the
/// debug-only `get_stack<DbRef>` guard).
#[test]
fn reserve_poison_fires_on_uninit_slot_read() {
    if std::env::var_os("LOFT_POISON").is_none() {
        eprintln!("skipped reserve-poison positive control (needs LOFT_POISON=1)");
        return;
    }
    let (mut state, _data) = build("fn test() { x = 1; assert(x == 1); }");
    // Reserve a frame region WITHOUT initialising it, then pop a slot the frame
    // never wrote.  Under reserve-poison every byte is drawn from the sentinel.
    state.reserve_frame(20);
    let word: u32 = state.get_stack::<u32>();
    let bytes = word.to_le_bytes();
    assert!(
        bytes.iter().all(|b| matches!(b, 0xEF | 0xBE | 0xAD | 0xDE)),
        "reserve-poison must fill an unwritten frame slot with the 0xDEADBEEF \
         sentinel; got {word:#010x} (bytes {bytes:02x?})"
    );
}

/// loft#1241 — a local the CONSTRUCT-shape move elision has erased must not hold a slot.
///
/// `x.field += src` where `src` is a literal-initialised local, dead after the append, folds
/// into `src`'s own construction: the element builds are re-pointed onto `x.field` and the
/// wrapper alloc, the view-def and the append are all dropped.  Nothing writes `src` after
/// that, so it is not a runtime local at all.
///
/// What kept it slotted was a stale `deps`: the element work-ref still said its store belonged
/// to `src`, and the scope pass declares a dep var so a borrower can name it.  That declaration
/// grants a slot no instruction writes, which is what @PLN120 A's store-span check reports —
/// and it reports it only under `-C debug-assertions=on`, so this asserts the FACT instead, in
/// a build that always runs.  The dep is the fact the rewrite changed
/// (`formal/ownership.md` O-Deps), and the work-ref's own dep is asserted beside the slot so a
/// fix that merely hid the slot would not pass.
///
/// `s` holding NO slot is itself the proof the fold still happens, so this cell doubles as its
/// own control: switch the elision off and `s` is a real local again and the assertion fails.
/// The RUN-COUNT control belongs to
/// `tests/scripts/an-appended-source-is-built-as-often-as-the-append-runs.loft` and is not
/// repeatable here — a loop cell reads the same slot table before and after the loft#1243 fix
/// (folded-with-a-stale-dep and declined-outright both leave `s` slotted), so a cell of that
/// shape would pass on the broken build and be a control in name only.
#[test]
fn a_retargeted_append_source_is_not_a_runtime_local() {
    let (_state, data) = build(
        "struct FvBag { c: vector<integer> }
fn test() {
    d = FvBag { c: [1] };
    s: vector<integer> = [7, 8];
    d.c += s;
    assert(len(d.c) == 3);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    let vars = &data.def(fn_d_nr).variables;
    let by_name = |want: &str| (0..vars.count()).find(|&v| vars.name(v) == want);
    let s = by_name("s").expect("no var `s` in n_test");
    assert_eq!(
        vars.stack(s),
        u16::MAX,
        "`s` is erased by the move elision, so it must hold no slot; got {}",
        vars.stack(s)
    );
    // The element work-ref borrows the CONTAINER now, which is what makes `s` nameless.
    let d = by_name("d").expect("no var `d` in n_test");
    let elm = (0..vars.count())
        .find(|&v| vars.name(v).starts_with("_elm") && vars.tp(v).depend().contains(&d))
        .expect("no element work-ref depending on `d` — the builds were not retargeted");
    assert!(
        !vars.tp(elm).depend().contains(&s),
        "`{}` still borrows the erased `s`: {:?}",
        vars.name(elm),
        vars.tp(elm).depend()
    );
}

/// loft#1335 — a fn-ref call's return records what it borrows in the CALLER's space, for
/// every kind of return that can borrow.
///
/// `fnref_result_type` maps the lambda's declared return deps (attribute indices, callee
/// space) through the actual arguments into frame variables; a hand-written list of shapes
/// did that for text, vector, struct and enum and handed every other kind back verbatim.  A
/// keyed return then reached the caller still naming attribute 0, which in the caller's frame
/// is whichever variable holds that number.  With a SCALAR parameter first, that variable is
/// the scalar, which carries no deps at all, so the function's own return read as OWNED — a
/// caller adopting it would adopt the argument's store.  The debug-assertions gate saw the
/// half of it a join makes visible (`dep-space violation: union of Attr deps with Frame
/// deps`); this asserts the FACT (`formal/ownership.md` O-Move), in a build that always runs:
/// the enclosing function's return must name the parameter the lambda's result borrows, and
/// nothing else.  `x` first is what makes the wrong index observable — with `bag` at index 0
/// the two spellings coincide.  The vector return is the control: it was mapped before, and
/// its answer is what the keyed and optional returns must match.  A tuple return is mapped
/// element-wise by the same code, but a named function cannot hand a fn-ref's tuple back as
/// its tail (`expected __tuple<…>, got (…) on return from block`, a standing refusal), so
/// that kind has no cell here.
#[test]
fn a_fn_ref_return_borrows_the_argument_in_the_callers_space_for_every_kind() {
    for (label, ret, body) in [
        ("vector (control)", "vector<integer>", "q.vs"),
        ("keyed", "hash<Fk1335[k]>", "q.m"),
        ("optional keyed", "hash<Fk1335[k]>?", "q.m"),
        ("optional struct", "Fbag1335?", "q"),
    ] {
        let script = format!(
            "struct Fk1335 {{ k: integer, v: integer }}
struct Fbag1335 {{ m: hash<Fk1335[k]>, vs: vector<integer> }}
fn pick(x: integer, bag: Fbag1335) -> {ret} {{
    assert(x == 1);
    h = fn(q: Fbag1335) -> {ret} {{ {body} }};
    h(bag)
}}
fn test() {{ bag = Fbag1335 {{ m: [Fk1335 {{ k: 3, v: 41 }}], vs: [5, 6] }}; pick(1, bag); assert(len(bag.m) == 1); }}"
        );
        let (_state, data) = build(&script);
        let pick = data.def_nr("n_pick");
        let attrs = data.def(pick).attributes();
        let bag_attr = attrs
            .iter()
            .position(|a| a.name == "bag")
            .expect("`bag` is a parameter of pick") as u16;
        let mut deps = data.def(pick).returned().depend();
        deps.sort_unstable();
        assert_eq!(
            deps,
            vec![bag_attr],
            "{label}: pick's return must record exactly the parameter its fn-ref result borrows \
             (attribute {bag_attr}, `bag`); got {deps:?} — attribute 0 is the scalar `x`, and an \
             empty list reads as an owned store the caller may adopt"
        );
    }
}
