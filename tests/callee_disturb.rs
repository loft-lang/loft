// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN203 rust-writer: the guard flips a Rust-writer switch to restore the defect it catches — its `--native` runs as `--native-rust` until then.
//! @PLN164 C3 (`@FR-B-Disturb`, `@FR-B-View`) — A DISTURBANCE ONE FRAME DOWN IS A DISTURBANCE:
//! a container a CALLEE grows or removes from ends the place a caller's view names, so the view
//! materialises exactly as it does when the same statement is written inline.
//!
//! `tests/scripts/164-callee-disturb.loft` carries the fifteen pairs and says the VALUES agree
//! on both backends; the corpus runner runs it.  What is pinned here is what a value comparison
//! cannot see: that the switch really does restore the defect (so the guard's `@falsified-at:`
//! is a measurement and not a claim), that the advice NAMES the callee — `(B-View)` promises
//! the author is told, and a reader told "`sc` grows" with no append anywhere in the function
//! is not told — and that the summary admits exactly the callees that disturb a parameter and
//! no others, which is the direction that would go wrong quietly: a widened summary
//! materialises views whose writes land today, and every one of those is a program losing its
//! meaning rather than a compiler losing an optimisation.
use std::path::{Path, PathBuf};
use std::process::Command;

const GUARD: &str = "tests/scripts/164-callee-disturb.loft";

fn guard() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(GUARD)
}

fn loft() -> Command {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "240")
        .env_remove("LOFT_NO_CALLEE_DISTURB");
    cmd
}

/// Run the guard in `mode` with `env`; the exit status, stdout and stderr.
fn run(mode: &str, env: &[(&str, &str)]) -> (bool, String, String) {
    let mut cmd = loft();
    cmd.arg(mode).arg(guard());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const STRICT: &[(&str, &str)] = &[
    ("LOFT_POISON", "1"),
    ("LOFT_POISON_CLAIM", "1"),
    ("LOFT_STRICT_STORES", "1"),
];
const STRICT_NATIVE: &[(&str, &str)] = &[
    ("LOFT_POISON", "1"),
    ("LOFT_POISON_CLAIM", "1"),
    ("LOFT_STRICT_STORES", "1"),
    ("LOFT_NATIVE_LEAK_CHECK", "1"),
];
const OFF: &[(&str, &str)] = &[("LOFT_NO_CALLEE_DISTURB", "1")];
/// The receipt's witness is a stale read through a block the realloc freed and the next claim
/// reused at once; `(H-LazyFree)` defers that reuse (the old bytes stay readable), so the exact
/// receipt is taken on the exact allocator and the default one is held to failing at all.
const OFF_EXACT: &[(&str, &str)] = &[("LOFT_NO_CALLEE_DISTURB", "1"), ("LOFT_NO_LAZY_FREE", "1")];

#[test]
fn the_pairs_agree_on_both_backends_under_every_falsifier() {
    let (ok, out, err) = run("--interpret", STRICT);
    assert!(ok, "the guard failed on --interpret:\n{err}");
    assert!(
        out.contains("16 pairs, inline and callee agree"),
        "the guard did not reach its last pair:\n{out}"
    );
    let (native_ok, native, native_err) = run("--native-rust", STRICT_NATIVE);
    assert!(native_ok, "the guard failed on --native:\n{native_err}");
    assert_eq!(out, native, "the two backends must print the same");
}

/// The switch restores the per-frame answer, and the guard then FAILS — which is what makes its
/// `@falsified-at:` receipt a measurement.  A guard that passed either way would be pinning
/// nothing.
#[test]
fn the_switch_restores_the_defect_the_guard_catches() {
    for mode in ["--interpret", "--native-rust"] {
        let (ok, _, err) = run(mode, OFF);
        assert!(
            !ok,
            "{mode} with LOFT_NO_CALLEE_DISTURB=1 must fail the guard — without that the guard \
             measures nothing:\n{err}"
        );
        // The stale read answers whatever the reused memory held — a value of the store's
        // reuse, not of the program, and no promise of the claim (a poisoned claim reads
        // another) — so the cell is pinned by WHERE it fails and that the answer is not 3.
        let (exact_ok, _, exact_err) = run(mode, OFF_EXACT);
        assert!(
            !exact_ok
                && exact_err.contains("c2 grow, realloc: callee ")
                && !exact_err.contains("c2 grow, realloc: callee 3, want 3"),
            "{mode} must fail on c2 with the stale read the unit removes:\n{exact_err}"
        );
    }
}

/// `(B-View)` promises the author is TOLD.  For a disturbance one frame down that promise needs
/// the callee's NAME: nothing in `c_grow_callee` appends to `sc`, so a reader given only
/// *"`sc` grows"* goes looking for a statement that is not there — the same failure the `Grown`
/// sentence was split off to avoid.
#[test]
fn the_advice_names_the_callee_the_disturbance_came_through() {
    let (_, _, err) = run("--interpret", &[]);
    assert!(
        err.contains(
            "in `c_write_callee`, `e` was copied out of `sc` because `sc` grows inside the call \
             to `grow_field` while `e` is in use"
        ),
        "a growth one frame down must name its callee:\n{err}"
    );
    assert!(
        err.contains(
            "in `c_rm_write_callee`, `e` was copied out of `sc` because `sc` is modified inside \
             the call to `remove_first` while `e` is in use"
        ),
        "a removal one frame down must name its callee:\n{err}"
    );
    // The INLINE twin keeps the sentence it always had: no call, no clause.
    assert!(
        err.contains(
            "in `c_write_inline`, `e` was copied out of `sc` because `sc` grows while `e` is in use"
        ),
        "an inline growth must not gain a callee clause:\n{err}"
    );
    // A view only READ after its copy reads the same bytes as the place would have: the copy
    // changes nothing the program computes, so it draws no advice.
    for read_only in ["c_grow_callee", "c_rm_moves_callee", "c_grow_inline"] {
        assert!(
            !err.contains(&format!("in `{read_only}`")),
            "`{read_only}` only reads its copied view, so it must draw no copy notice:\n{err}"
        );
    }
    // Every control must be silent: an advice line there would be asserting that writes stop
    // reaching the container, over a binding the cells prove still aliases.
    for quiet in [
        "c_alias_callee",
        "c_other_callee",
        "c_sibling_callee",
        "c_unrelated_callee",
    ] {
        assert!(
            !err.contains(&format!("in `{quiet}`")),
            "`{quiet}` still aliases, so it must draw no copy notice:\n{err}"
        );
    }
}

/// The advice names a `match` payload binding as the author WROTE it.  The binding is lowered
/// to `xs#1` (a second binding of one name), and the sentence used to print that — a name the
/// author cannot find in the source or type back.
#[test]
fn the_advice_names_a_payload_binding_as_written() {
    let dir = std::env::temp_dir().join(format!("loft-disturb-name-{}", std::process::id()));
    loft::file_access::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join("payload.loft");
    loft::file_access::write(
        &file,
        "struct N { xs: vector<integer> }\n\
         struct S { nodes: vector<N> }\n\
         fn add(s: S, n: N) -> integer { s.nodes += [n]; len(s.nodes) - 1 }\n\
         fn live_after(s: S) -> integer {\n\
           match s.nodes[0] { N { xs } => { k = add(s, N { xs: [] }); xs += [7]; k + len(xs) } }\n\
         }\n\
         fn main() { s = S { nodes: [N { xs: [1, 2] }] }; assert(live_after(s) == 4, \"k + len\"); }\n",
    )
    .expect("write probe");
    let out = loft()
        .arg("--interpret")
        .arg(&file)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "the probe must run:\n{err}");
    assert!(
        err.contains(
            "in `live_after`, `xs` was copied out of `s` because `s` grows inside the call to `add`"
        ),
        "the advice must name the binding `xs`:\n{err}"
    );
    assert!(
        !err.contains("xs#"),
        "no internal binding name may reach the author:\n{err}"
    );
}

/// The summary admits exactly the callees that disturb a caller's container through a parameter.
/// `mk` is the control that matters most: it GROWS `__retbuf.els` building its own result, and
/// counting a hidden return buffer as a caller's container would report a disturbance at every
/// call to every record-returning function in the program.
#[test]
fn the_summary_admits_the_disturbing_callees_and_no_others() {
    let mut cmd = loft();
    cmd.arg("--interpret")
        .arg(guard())
        .env("LOFT_TRACE_DISTURB", "1");
    let out = cmd.output().expect("spawn loft");
    let err = String::from_utf8_lossy(&out.stderr);
    let mut named: Vec<&str> = err
        .lines()
        .filter_map(|l| l.strip_prefix("[disturb] "))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    named.sort_unstable();
    named.dedup();
    assert_eq!(
        named,
        [
            // Reached through the call graph: each one forwards its own parameter downward.
            "n_c_param_callee",
            // Disturbs its own parameter inline, which is the same fact about its body.
            "n_c_param_inline",
            "n_grow_deep",
            "n_grow_field",
            "n_grow_sibling",
            "n_grow_vec",
            // `sc.els = [x]` REBUILDS the field: the views inside it end, as on a growth
            // (`(B-Disturb)`), so the rebuild is a disturbance of the caller's container.
            "n_rebuild_field",
            "n_remove_first",
            "n_remove_last",
            // A METHOD is a callee like any other (loft#2267): `vector.clear` removes every
            // element of its `self`.  `sort` and `sort_by` write each element whole
            // (`self[at] = e`), which releases what the old element owned: a view OF an element
            // survives, a view deeper than one ends (`(B-Disturb)`, loft#2391).  A promoted
            // result buffer (`File.files`' `result`) is hidden, so it is not here.
            "t_6vector_clear",
            "t_6vector_sort",
            "t_6vector_sort_by",
        ],
        "the summary must name every definition that disturbs a caller's container through a \
         parameter, and nothing else.  The absences are the load-bearing half: `n_mk` GROWS \
         `__retbuf.els` building its own result, and counting a hidden return buffer would \
         report a disturbance at every call to every record-returning function; \
         `n_touch_nothing` and `n_write_other_field` do nothing to a container at all:\n{err}"
    );
}

/// loft#2349 (`@FR-B-View`: a view materialises only where it is LIVE ACROSS the event) — a
/// call's arguments are evaluated before its callee runs, so a view read only into an argument
/// that cannot carry it into the callee — a record literal built from it, a scalar parameter —
/// is dead when the callee grows, removes from or reassigns the container: no copy, no advice.
/// Read as disturb-then-use, `add(s, N { xs: xs })` deep-copied `xs` and told the author it was
/// "in use" across the growth.  The other arguments keep both — the view itself, a text built
/// from it, a read after the call (the next statement or the same one), a second disturbing
/// call, a loop — and those notices must survive.  Values (every callee reads what it was
/// handed after its disturbance) are asserted under the strict falsifiers on both backends;
/// the suite cell `rules/binding/B-Disturb/2349-…` carries the wider value matrix.
const ARGUMENT_READS: &str = r#"
struct N { xs: vector<integer> }
struct S { nodes: vector<N> }
fn mk() -> S { S { nodes: [N { xs: [1, 2] }, N { xs: [3, 4] }] } }
fn grow(s: S) { for k in 0..20 { s.nodes += [N { xs: [100 + k] }]; } }
fn add(s: S, n: N) -> integer { grow(s); len(s.nodes) * 100 + n.xs[0] * 10 + n.xs[1] }
fn add_vec(s: S, v: vector<integer>) -> integer { grow(s); len(s.nodes) * 100 + v[0] * 10 + v[1] }
fn add_int(s: S, k: integer) -> integer { grow(s); len(s.nodes) * 100 + k }
fn add_text(s: S, t: text) -> integer { grow(s); len(s.nodes) * 100 + len(t) }
fn rm(s: S, n: N) -> integer { s.nodes.remove(0); len(s.nodes) * 100 + n.xs[0] * 10 + n.xs[1] }
fn reset(s: &S, n: N) -> integer { s = S { nodes: [N { xs: [9] }] }; len(s.nodes) * 100 + n.xs[0] * 10 + n.xs[1] }
fn g_lit(s: S) -> integer { match s.nodes[0] { N { xs } => add(s, N { xs: xs }) } }
fn g_elem(s: S) -> integer { match s.nodes[0] { N { xs } => add_int(s, xs[1]) } }
fn g_len(s: S) -> integer { match s.nodes[0] { N { xs } => add_int(s, len(xs)) } }
fn g_plain(s: S) -> integer { nd = s.nodes[0]; add(s, N { xs: nd.xs }) }
fn r_lit(s: S) -> integer { match s.nodes[0] { N { xs } => rm(s, N { xs: xs }) } }
fn a_lit(s: &S) -> integer { match s.nodes[0] { N { xs } => reset(s, N { xs: xs }) } }
fn k_view(s: S) -> integer { match s.nodes[0] { N { xs } => add_vec(s, xs) } }
fn k_text(s: S) -> integer { match s.nodes[0] { N { xs } => add_text(s, "{xs}") } }
fn k_next(s: S) -> integer { match s.nodes[0] { N { xs } => { r = add(s, N { xs: xs }); r + xs[1] } } }
fn k_same(s: S) -> integer { match s.nodes[0] { N { xs } => add(s, N { xs: xs }) + xs[1] } }
fn k_two(s: S) -> integer { match s.nodes[0] { N { xs } => add(s, N { xs: xs }) + add(s, N { xs: xs }) } }
fn k_loop(s: S) -> integer { t = 0; match s.nodes[0] { N { xs } => { for _ in 0..2 { t += add(s, N { xs: xs }); } } } t }
fn k_rm_next(s: S) -> integer { match s.nodes[0] { N { xs } => { r = rm(s, N { xs: xs }); r * 10 + xs[1] } } }
fn main() {
  s1 = mk(); s2 = mk(); s3 = mk(); s4 = mk(); s5 = mk(); s6 = mk(); s7 = mk();
  s8 = mk(); s9 = mk(); s10 = mk(); s11 = mk(); s12 = mk(); s13 = mk();
  print("{g_lit(s1)} {g_elem(s2)} {g_len(s3)} {g_plain(s4)} {r_lit(s5)} {a_lit(s6)} {s6.nodes[0].xs} ");
  print("{k_view(s7)} {k_text(s8)} {k_next(s9)} {k_same(s10)} {k_two(s11)} {k_loop(s12)} {k_rm_next(s13)}\n");
}
"#;

fn assert_argument_reads(mode: &str, env: &[(&str, &str)]) {
    let dir = std::env::temp_dir().join(format!(
        "loft_2349_argument_reads_{}",
        mode.trim_matches('-')
    ));
    loft::file_access::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("argument_reads.loft");
    loft::file_access::write(&path, ARGUMENT_READS).expect("write program");
    let mut cmd = loft();
    // The trace names every view copied out of its container; the advice names only one
    // something writes through, and these views are only read.
    cmd.arg(mode).arg(&path).env("LOFT_TRACE_DISTURB", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "[{mode}] the program must run clean\n{stdout}\n---\n{err}"
    );
    // grow: 2 + 20 nodes (a second grow 42); rm leaves 1; reset leaves 1.  A dead argument
    // read answers the record it was built from; `k_text` counts "[1,2]".
    assert!(
        stdout.contains("2212 2202 2202 2212 112 112 [9] 2212 2205 2214 2214 6424 6424 1122"),
        "[{mode}] the program must answer its hand-computed values\n{stdout}"
    );
    for dead in ["g_lit", "g_elem", "g_len", "g_plain", "r_lit", "a_lit"] {
        assert!(
            !err.contains(&format!("[materialise] {dead} ")),
            "[{mode}] `{dead}` reads its view only into an argument the callee cannot keep — \
             no copy\n{err}"
        );
    }
    for live in [
        "k_view",
        "k_text",
        "k_next",
        "k_same",
        "k_two",
        "k_loop",
        "k_rm_next",
    ] {
        assert!(
            err.contains(&format!("[materialise] {live} `xs`")),
            "[{mode}] `{live}` holds its view across the callee's disturbance — the copy \
             stays\n{err}"
        );
        // Only read after the copy: the author is not told.
        assert!(
            !err.contains(&format!("in `{live}`,")),
            "[{mode}] `{live}` only reads its copied view — no copy notice\n{err}"
        );
    }
}

#[test]
fn a_view_read_only_into_an_argument_is_not_live_across_the_callee_interpret() {
    assert_argument_reads("--interpret", STRICT);
}

#[test]
fn a_view_read_only_into_an_argument_is_not_live_across_the_callee_native_c() {
    assert_argument_reads("--native-c", STRICT_NATIVE);
}

/// loft#2358 (`@FR-B-Disturb`, `@FR-C-Ref`) — a `&` PARAMETER is a container its views point
/// into, as a local is: reassigning it, in a callee it is passed on to or inline, ends the
/// places inside it, so a view live across that is copied at its bind and the author is told.
/// The walk asked `.base()` whether the reassigned variable held a record, which keeps the
/// `&`, so the parameter spelling alone kept its alias and read the released record.  The
/// control reads its view before the reassignment and must stay silent.
const REF_PARAM_REASSIGN: &str = r#"
struct N { xs: vector<integer> }
struct S { nodes: vector<N> }
fn mk() -> S { S { nodes: [N { xs: [1, 2] }, N { xs: [3, 4] }] } }
fn fresh() -> S { S { nodes: [N { xs: [9] }] } }
fn reset(s: &S) -> integer { s = fresh(); len(s.nodes) * 100 }
fn p_callee(s: &S) -> integer { xs = s.nodes[0].xs; r = reset(s); r + xs[0] * 10 + xs[1] }
fn p_inline(s: &S) -> integer { xs = s.nodes[0].xs; s = fresh(); len(s.nodes) * 100 + xs[0] * 10 + xs[1] }
fn p_loop(s: &S) -> integer { xs = s.nodes[0].xs; t = 0; for _ in 0..2 { t += reset(s) + xs[0]; } t }
fn c_dead(s: &S) -> integer { xs = s.nodes[0].xs; a = xs[0] * 10 + xs[1]; r = reset(s); r + a }
fn p_write(s: &S) -> integer { xs = s.nodes[0].xs; r = reset(s); xs += [5]; r + len(xs) }
fn main() {
  s1 = mk(); s2 = mk(); s3 = mk(); s4 = mk(); s5 = mk();
  print("{p_callee(s1)} {p_inline(s2)} {p_loop(s3)} {c_dead(s4)} {s1.nodes[0].xs} {p_write(s5)}\n");
}
"#;

fn assert_ref_param_reassign(mode: &str, env: &[(&str, &str)]) {
    let dir = std::env::temp_dir().join(format!(
        "loft_2358_ref_param_reassign_{}",
        mode.trim_matches('-')
    ));
    loft::file_access::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("ref_param_reassign.loft");
    loft::file_access::write(&path, REF_PARAM_REASSIGN).expect("write program");
    let mut cmd = loft();
    cmd.arg(mode).arg(&path);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "[{mode}] the program must run clean\n{stdout}\n---\n{err}"
    );
    // reset leaves one node (100); each view holds the record as it was at its bind, and
    // `p_write`'s append lands in its copy: [1, 2, 5].
    assert!(
        stdout.contains("112 112 202 112 [9] 103"),
        "[{mode}] the program must answer its hand-computed values\n{stdout}"
    );
    assert!(
        err.contains(
            "in `p_write`, `xs` was copied out of `s` because `s` is reassigned while `xs` is in \
             use"
        ),
        "[{mode}] `p_write` writes through its view after `s` is reassigned — the copy is owed \
         and the author is told\n{err}"
    );
    // The copy is owed in these too (the values above prove it), but nothing writes through
    // the view, so the copy changes nothing the program computes and draws no advice.
    for read_only in ["p_callee", "p_inline", "p_loop"] {
        assert!(
            !err.contains(&format!("in `{read_only}`,")),
            "[{mode}] `{read_only}` only reads its copied view — no copy notice\n{err}"
        );
    }
    assert!(
        !err.contains("in `c_dead`,"),
        "[{mode}] `c_dead` reads its view before the reassignment — no copy\n{err}"
    );
}

#[test]
fn a_view_of_a_ref_parameter_materialises_when_it_is_reassigned_interpret() {
    assert_ref_param_reassign("--interpret", STRICT);
}

#[test]
fn a_view_of_a_ref_parameter_materialises_when_it_is_reassigned_native_c() {
    assert_ref_param_reassign("--native-c", STRICT_NATIVE);
}
