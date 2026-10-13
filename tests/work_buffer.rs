// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-WorkBuffer` — the admission of a non-escaping vector local, PINNED on the IR of the
//! cells (`tests/scripts/a-non-escaping-vector-local-is-a-caller-buffer.loft`): a promoted
//! local is an ARGUMENT of its function (the IR header names it beside the parameters), a
//! declined one stays a local.  The cells' values pass on either form, so only this pin sees
//! an admission drift; the null road is run here too, since no source can spell it.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-non-escaping-vector-local-is-a-caller-buffer.loft";

/// Per function: the locals promoted to a caller buffer (the rewrite ON; OFF: always none).
const EXPECTED: &[(&str, &[&str])] = &[
    ("n_c1", &["v"]),        // the probe
    ("n_c2", &["v"]),        // recursion
    ("n_c3", &["xs", "ks"]), // two buffers
    ("n_c4", &[]),           // declared inside a loop: @FR-R-LoopBuffer / @FR-R-LitHoist keep it
    ("n_c5", &["v"]),        // every in-place use, the walk's alias included
    ("n_c6", &[]), // beside a return buffer: the swap would renumber attr-space return deps
    ("n_c7", &["v"]), // handed by value to a callee answering a scalar
    ("n_c7b", &["v"]), // the callee answers its parameter — as a copy through its buffer
    ("n_c7c", &[]), // handed to a `&` parameter
    ("n_c8", &[]), // a text element
    ("n_c9", &[]), // assigned a second time
    ("n_c10", &[]), // v copied whole into the return buffer (@FR-R-RetAdopt), w a copy-bind
    ("t_3Acc_bump", &["v"]), // a method
    ("n_c12w", &[]), // a par worker
    ("n_c12", &["out"]), // the par caller's own collector
    ("n_c13", &["v"]), // an early return
    ("n_c15", &["acc"]), // a buffer-holding caller
    ("n_c16", &["v"]), // beside a parse-time return buffer of its own
    ("n_c17leaf", &["v"]),
    ("n_c20c", &["v"]),
];

/// Per function: how many of its work-refs the transitive clause promotes onward, and how
/// many it keeps (a callee on a cycle with the function).
const EXPECTED_REFS: &[(&str, usize, usize)] = &[
    ("n_c2", 0, 1),   // the self-call's buffer: one per activation
    ("n_c15", 1, 0),  // c1's buffer, handed down from main
    ("n_c17", 1, 0),  // the wrapper
    ("n_c17o", 1, 0), // the wrapper's caller: the buffer climbs to main
    // The leaf's buffer onward, and the self-call takes that same forwarded buffer from the
    // frame's pool (loft#1697): a forwarded buffer holds nothing across a call, so the inner
    // activation may use it.  Per site, the self-call minted a buffer of its own every call.
    ("n_c18", 1, 0),
    // Three calls, two callees, each taking one `vector<integer>`: calls in a frame never
    // overlap, so the pool gives them ONE buffer (loft#1697) — per site it was three.
    ("n_c19", 1, 0),
    ("n_c20w", 0, 1), // its callee calls back into it with the buffer live
];

fn loft() -> Command {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_WORK_BUFFER")
        .env_remove("LOFT_WORK_BUFFER_NULL");
    cmd
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

/// The IR and the trace of the cells, with `env` on top of the pin's own.
fn introspect(env: &[(&str, &str)]) -> (String, String) {
    let mut cmd = loft();
    cmd.arg("introspect")
        .arg(cells())
        .env("LOFT_TRACE_WORK_BUFFER", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    let ir = String::from_utf8_lossy(&out.stdout).into_owned();
    let trace = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        ir.contains("fn n_c1("),
        "no IR printed (exit {:?}): {trace}",
        out.status
    );
    (ir, trace)
}

/// The locals the trace names as promoted in `name`, in trace order.
fn promoted_in(trace: &str, name: &str) -> Vec<String> {
    let head = format!("[work-buffer] fn={name} local=");
    trace
        .lines()
        .filter_map(|l| l.strip_prefix(&head))
        .filter_map(|rest| {
            rest.strip_suffix(" PROMOTED to a caller buffer")
                .map(str::to_string)
        })
        .collect()
}

/// The work-refs of `name` the trace reports promoted onward and kept, each named once
/// (every round of the fixpoint judges a kept ref again).
fn refs_in(trace: &str, name: &str) -> (Vec<String>, Vec<String>) {
    let head = format!("[work-buffer] fn={name} ref=");
    let (mut promoted, mut kept) = (Vec::new(), Vec::new());
    for rest in trace.lines().filter_map(|l| l.strip_prefix(&head)) {
        let (r, verdict) = rest.split_once(' ').unwrap_or((rest, ""));
        let list = if verdict == "PROMOTED onward" {
            &mut promoted
        } else {
            &mut kept
        };
        if !list.iter().any(|x| x == r) {
            list.push(r.to_string());
        }
    }
    (promoted, kept)
}

/// The IR header of `fn <name>(…)`: its parameter list as printed.
fn header_of<'a>(ir: &'a str, name: &str) -> &'a str {
    let head = format!("\nfn {name}(");
    let start = ir
        .find(&head)
        .unwrap_or_else(|| panic!("{name} was not printed"));
    let rest = &ir[start + head.len()..];
    let end = rest.find(')').unwrap_or(rest.len());
    &rest[..end]
}

/// The header names `local` as a parameter — in the bytecode spelling (`v:vector<…>`) or,
/// for a method printed only in the native section, the emitted one (`mut var_v: DbRef`).
fn names_parameter(header: &str, local: &str) -> bool {
    header
        .split(", ")
        .any(|p| p.starts_with(&format!("{local}:")) || p.starts_with(&format!("mut var_{local}:")))
}

#[test]
fn a_non_escaping_local_is_promoted_and_anything_else_keeps_its_store() {
    let (ir, trace) = introspect(&[]);
    for (name, expected) in EXPECTED {
        assert_eq!(
            promoted_in(&trace, name),
            expected.to_vec(),
            "{name}: the promoted locals — the cell's admission moved"
        );
        // And the promotion is real: a promoted local is a PARAMETER of its function.
        for local in *expected {
            assert!(
                names_parameter(header_of(&ir, name), local),
                "{name}: `{local}` was reported promoted but is no parameter"
            );
        }
    }
}

/// The transitive clause: a promoted work-ref is a parameter of its function, and every
/// function the cells hold that is not pinned here has no work-ref promoted.
#[test]
fn a_handed_down_buffer_climbs_to_the_frame_that_loops() {
    let (ir, trace) = introspect(&[]);
    for (name, promoted, kept) in EXPECTED_REFS {
        let (p, k) = refs_in(&trace, name);
        assert_eq!(
            (p.len(), k.len()),
            (*promoted, *kept),
            "{name}: work-refs promoted onward / kept — promoted {p:?}, kept {k:?}"
        );
        for r in &p {
            assert!(
                names_parameter(header_of(&ir, name), r),
                "{name}: `{r}` was reported promoted onward but is no parameter"
            );
        }
    }
    for line in trace.lines().filter(|l| l.ends_with(" PROMOTED onward")) {
        let name = line
            .strip_prefix("[work-buffer] fn=")
            .and_then(|r| r.split_once(' '))
            .map_or("", |(n, _)| n);
        assert!(
            EXPECTED_REFS.iter().any(|(n, _, _)| *n == name),
            "an unpinned function promoted a work-ref onward: {line}"
        );
    }
}

#[test]
fn the_switch_keeps_every_local() {
    let (ir, trace) = introspect(&[("LOFT_NO_WORK_BUFFER", "1")]);
    assert!(
        !trace.contains("PROMOTED"),
        "LOFT_NO_WORK_BUFFER=1 must promote nothing:\n{trace}"
    );
    for (name, expected) in EXPECTED {
        for local in *expected {
            assert!(
                !names_parameter(header_of(&ir, name), local),
                "{name}: LOFT_NO_WORK_BUFFER=1 must leave `{local}` a local"
            );
        }
    }
}

/// The null road: every caller-side buffer left at the sentinel, so each promoted callee
/// mints a store of its own and releases it at exit — the same values, no leak.
#[test]
fn the_null_road_answers_the_same_and_leaks_nothing() {
    let out = loft()
        .arg("--interpret")
        .arg(cells())
        .env("LOFT_WORK_BUFFER_NULL", "1")
        .env("LOFT_STORES", "warn")
        .output()
        .expect("spawn loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("done"),
        "the cells failed on the null road (exit {:?}):\n{stdout}\n{stderr}",
        out.status
    );
    assert!(
        !stderr.contains("not freed") && !stderr.contains("leak"),
        "a store leaked on the null road:\n{stderr}"
    );
}

// ── the result clause ──────────────────────────────────────────────────────────────────

const RESULT_CELLS: &str = "tests/scripts/a-result-read-in-place-is-a-caller-buffer.loft";

/// Per function: the result refs the result clause promotes (the rewrite ON; OFF: none).
const EXPECTED_RESULTS: &[(&str, &[&str])] = &[
    ("n_weigh", &["__ref_1"]),            // the probe: an index walk
    ("n_weigh_q", &["__ref_1"]),          // the `?` read
    ("n_sizes", &["__ref_1"]),            // a `for` over the result
    ("n_heavy", &["__ref_1"]),            // text elements
    ("n_sum_evens", &["__ref_1"]),        // scalar elements
    ("n_cross", &["__ref_1", "__ref_2"]), // two results alive together
    ("n_styles", &["__ref_1"]),
    ("n_first_word", &[]), // answers a text: its text buffer is numbered after the ref
    ("n_note", &["__ref_1"]), // an element's text stored in the caller's record
    ("n_deep", &["__ref_1"]), // calls itself: that is what runs it again
    ("n_pass", &[]),       // the result is the answer: the call fills the return buffer
    ("n_grown", &[]),      // appended to
    ("n_bag", &[]),        // stored whole in a record
    ("n_via", &[]),        // handed to a callee
    ("n_bump", &[]),       // an element written
    ("n_guarded", &["__ref_1"]), // an exit before the call
    ("n_once", &[]),       // no call of it stands in a loop
    ("n_c14w", &[]),       // a par worker has no result of its own
    ("n_mix", &["__ref_1"]),
];

fn result_introspect(env: &[(&str, &str)]) -> (String, String) {
    let mut cmd = loft();
    cmd.arg("introspect")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(RESULT_CELLS))
        .env_remove("LOFT_NO_RESULT_BUFFER")
        .env("LOFT_TRACE_WORK_BUFFER", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    let ir = String::from_utf8_lossy(&out.stdout).into_owned();
    let trace = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        ir.contains("fn n_weigh("),
        "no IR printed (exit {:?}): {trace}",
        out.status
    );
    (ir, trace)
}

/// The IR header line of `fn <name>(…)`, whole: a record element type prints a `)` of its
/// own (`vector<ref(Run)>`), which is where `header_of` stops.
fn header_line_of<'a>(ir: &'a str, name: &str) -> &'a str {
    let head = format!("\nfn {name}(");
    let start = ir
        .find(&head)
        .unwrap_or_else(|| panic!("{name} was not printed"));
    let rest = &ir[start + head.len()..];
    &rest[..rest.find('\n').unwrap_or(rest.len())]
}

/// The result refs the trace names as promoted in `name`, in trace order.
fn results_in(trace: &str, name: &str) -> Vec<String> {
    let head = format!("[work-buffer] fn={name} result=");
    trace
        .lines()
        .filter_map(|l| l.strip_prefix(&head))
        .filter_map(|rest| {
            rest.strip_suffix(" PROMOTED to a caller buffer")
                .map(str::to_string)
        })
        .collect()
}

#[test]
fn a_result_only_read_is_promoted_and_any_other_use_keeps_its_mint() {
    let (ir, trace) = result_introspect(&[]);
    for (name, expected) in EXPECTED_RESULTS {
        assert_eq!(
            results_in(&trace, name),
            expected.to_vec(),
            "{name}: the promoted result refs — the cell's admission moved"
        );
        for r in *expected {
            assert!(
                names_parameter(header_line_of(&ir, name), r),
                "{name}: `{r}` was reported promoted but is no parameter"
            );
        }
    }
    // Every other function of the cells keeps its refs: an admission nobody pinned is a move.
    for line in trace
        .lines()
        .filter(|l| l.contains(" result=") && l.ends_with(" PROMOTED to a caller buffer"))
    {
        let name = line
            .strip_prefix("[work-buffer] fn=")
            .and_then(|r| r.split_once(' '))
            .map_or("", |(n, _)| n);
        assert!(
            EXPECTED_RESULTS.iter().any(|(n, _)| *n == name),
            "an unpinned function had a result ref promoted: {line}"
        );
    }
}

#[test]
fn the_result_switch_keeps_every_mint() {
    let (ir, trace) = result_introspect(&[("LOFT_NO_RESULT_BUFFER", "1")]);
    assert!(
        !trace.contains(" result="),
        "LOFT_NO_RESULT_BUFFER=1 must judge no result ref:\n{trace}"
    );
    for (name, expected) in EXPECTED_RESULTS {
        for r in *expected {
            assert!(
                !names_parameter(header_line_of(&ir, name), r),
                "{name}: LOFT_NO_RESULT_BUFFER=1 must leave `{r}` a local"
            );
        }
    }
}

/// The null road of a promoted reader: handed no buffer it mints its own and releases it
/// at its exit — the same values, nothing left alive.
#[test]
fn a_reader_handed_no_buffer_answers_the_same_and_leaks_nothing() {
    let out = loft()
        .arg("--interpret")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(RESULT_CELLS))
        .env_remove("LOFT_NO_RESULT_BUFFER")
        .env("LOFT_WORK_BUFFER_NULL", "1")
        .env("LOFT_STORES", "warn")
        .output()
        .expect("spawn loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("done"),
        "the cells failed on the null road (exit {:?}):\n{stdout}\n{stderr}",
        out.status
    );
    assert!(
        !stderr.contains("not freed") && !stderr.contains("leak"),
        "a store leaked on the null road:\n{stderr}"
    );
}

// ── the function-value clause ──────────────────────────────────────────────────────────

const FNREF_CELLS: &str =
    "tests/scripts/2001-a-work-buffer-through-a-function-value-is-its-own-store.loft";

fn fnref_emit(env: &[(&str, &str)], tag: &str) -> String {
    let out = std::env::temp_dir().join(format!("loft_fnref_work_{tag}.rs"));
    let mut cmd = loft();
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(FNREF_CELLS))
        .env_remove("LOFT_NO_FNREF_WORK_BUFFER");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let res = cmd.output().expect("spawn loft --native-emit");
    let rust = loft::file_access::read_to_string(&out).unwrap_or_else(|_| {
        panic!(
            "no Rust emitted (exit {:?}): {}",
            res.status,
            String::from_utf8_lossy(&res.stderr)
        )
    });
    let _ = loft::file_access::remove_file(&out);
    rust
}

/// A dispatch arm hands a candidate's work buffer a store the frame lends, and never the
/// return buffer's (loft#2001); the frames that lend are the ones that call through a
/// function value.
#[test]
fn a_function_value_call_lends_each_work_buffer_its_own_store() {
    let rust = fnref_emit(&[], "on");
    assert!(
        !rust.contains("__vc_hbuf, __vc_hbuf"),
        "a work buffer was handed the return buffer's store"
    );
    for (arm, lends) in [
        ("n_summed(cell, _farg_0, __vc_hbuf, __fwb.lend(", 1),
        ("n_tagged(cell, _farg_0, __vc_hbuf, __fwb.lend(", 1),
        ("n_tally(cell, _farg_0, __fwb.lend(", 1),
    ] {
        assert!(
            rust.matches(arm).count() >= lends,
            "no dispatch arm lends as `{arm}…`"
        );
    }
    // `two` takes two work buffers of two element types: two wrapper types, position 0 each.
    let two = rust
        .lines()
        .find(|l| l.contains("=> n_two(cell, _farg_0, __fwb.lend("))
        .expect("the arm for `two`");
    let arm = &two[two.find("=> n_two(").unwrap_or(0) + 3..];
    let arm = &arm[..arm.find(" => ").unwrap_or(arm.len())];
    assert_eq!(arm.matches("__fwb.lend(").count(), 2, "{arm}");
    assert_eq!(arm.matches("_i32, 0_u32)").count(), 2, "{arm}");
    for name in ["n_apply", "n_show_s", "n_show_r", "n_pick"] {
        let body = &rust[rust
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("{name} was not emitted"))..];
        let body = &body[..body[1..].find("\nfn ").map_or(body.len(), |i| i + 1)];
        assert!(
            body.contains("let mut __fwb = codegen_runtime::FnRefWork::new(cell);"),
            "{name} calls through a function value and declares no lender"
        );
    }
    let tally = &rust[rust.find("fn n_tally(").expect("n_tally")..];
    let tally = &tally[..tally[1..].find("\nfn ").map_or(tally.len(), |i| i + 1)];
    assert!(
        !tally.contains("__fwb"),
        "n_tally calls through no function value and must declare no lender"
    );
}

#[test]
fn the_function_value_switch_hands_the_sentinel() {
    let rust = fnref_emit(&[("LOFT_NO_FNREF_WORK_BUFFER", "1")], "off");
    assert!(
        !rust.contains("__fwb"),
        "LOFT_NO_FNREF_WORK_BUFFER=1 must lend nothing"
    );
    assert!(
        !rust.contains("__vc_hbuf, __vc_hbuf"),
        "a work buffer was handed the return buffer's store"
    );
    assert!(
        rust.contains("n_summed(cell, _farg_0, __vc_hbuf, loft::keys::DbRef::NULL)"),
        "with the switch a work buffer takes the null sentinel"
    );
}
