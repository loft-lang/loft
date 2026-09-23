// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN164 B1b (`@FR-O-Buffer`, `@FR-R-Reuse`, `@FR-O-Move`) — A CALLEE HANDED A LIVE BUFFER.
//! The caller's record-buffer pool takes the buffer of a call whose result a local adopts, so
//! a callee that promotes a local onto its buffer is handed the caller's store.  The cell
//! corpus (`plans/164-activation-arena/bytecode-comparisons/B1b-adopt-buffer-reuse-cells.loft`)
//! says the VALUES hold on both backends under the store falsifiers; this pins the PROTOCOL —
//! the entry witness is minted and guards the promoted buffer's frees on both backends, the
//! pool enrolls the adopting buffers, a bind after an `if` pre-init adopts, a body wrapped
//! around a hoisted result still takes the pool, a value-record scanner keeps its registers,
//! and `LOFT_NO_ADOPT_BUFFER_REUSE=1` restores B1 — and the store census the change buys.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/164-activation-arena/bytecode-comparisons/B1b-adopt-buffer-reuse-cells.loft";

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_ADOPT_BUFFER_REUSE")
        .env_remove("LOFT_NO_ADOPT_FIRST_BIND");
    cmd
}

fn with_env(cmd: &mut Command, env: &[(&str, &str)]) {
    for (k, v) in env {
        cmd.env(k, v);
    }
}

fn emit(out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd = loft();
    cmd.arg("--native-emit").arg(out).arg(cells());
    with_env(&mut cmd, env);
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read_to_string(out).expect("read the emitted Rust")
}

fn introspect(env: &[(&str, &str)]) -> String {
    let mut cmd = loft();
    cmd.arg("introspect").arg(cells());
    with_env(&mut cmd, env);
    let out = cmd.output().expect("spawn loft introspect");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The first listing of `fn <name>(` up to the next top-level `fn` — in an introspect
/// listing that is the IR and bytecode, in an emission the Rust body.
fn section<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} is not in the listing"));
    let rest = &text[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

fn store_mints(mode: &str, env: &[(&str, &str)]) -> usize {
    let mut cmd = loft();
    cmd.arg(mode).arg(cells()).env("LOFT_TRACE_DB", "1");
    with_env(&mut cmd, env);
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.contains("OpDatabase") && l.contains("#65535"))
        .count()
}

const OFF: &[(&str, &str)] = &[("LOFT_NO_ADOPT_BUFFER_REUSE", "1")];

#[test]
fn the_entry_witness_guards_the_promoted_buffer() {
    let ir = introspect(&[]);
    // The rebind of a promoted buffer (D-own-43): the snapshot, and the interpreter's
    // displaced free guarded by it, ahead of the call.
    let r1 = section(&ir, "n_render_lit_then_call");
    assert!(
        r1.contains("__rbw_cv(0):ref(Canvas)[\"__rbw_cv\"] = OpRefAlias(cv(0));"),
        "render_lit_then_call: the entry witness is minted"
    );
    let free = r1
        .find("FreeRefIfDistinct(placeholder")
        .expect("render_lit_then_call: the rebind's displaced free is guarded");
    let call = r1
        .find("fn=n_alloc_canvas)")
        .expect("render_lit_then_call calls alloc_canvas");
    assert!(
        free < call,
        "render_lit_then_call: the guarded free precedes the call"
    );
    // A literal exit beside a chain writes the buffer the chain renamed (B2 unit 1 over a
    // chain), so it answers the caller's store and frees nothing ...
    let scan = section(&ir, "n_scan_mk");
    assert!(
        scan.contains("return {#Object(8):ref(Mk)[\"__ref_1\"]"),
        "scan_mk: the literal exit builds into the chain's buffer"
    );
    // ... and where it builds a store of its own, the buffer's exit free declines on the
    // entry store.
    let own = introspect(&[("LOFT_NO_LITERAL_EXIT_BUFFER", "1")]);
    assert!(
        section(&own, "n_scan_mk").contains(
            "if OpDistinctStore(__ref_1(0), __rbw___ref_1(0)) OpFreeRefIfDistinct(__ref_1(0), __ret_1("
        ),
        "scan_mk: the literal exit's free is guarded by the entry witness"
    );
}

#[test]
fn the_pool_takes_an_adopting_callees_buffer() {
    let ir = introspect(&[]);
    assert!(
        section(&ir, "n_r9").contains("if OpRefIsNull(__ref_1(1)) {"),
        "r9: the chain callee's buffer is pooled"
    );
    // A body the scan wrapped around its hoisted result (`Insert([Set(sc, null), Block])`).
    let scene = section(&ir, "n_scene_at");
    assert!(
        scene.contains("sc(1):ref(Sk) = null") && scene.contains("if OpRefIsNull(__ref_"),
        "scene_at: the wrapped body still takes the pool"
    );
}

#[test]
fn the_switch_restores_b1() {
    let ir = introspect(OFF);
    assert!(
        !ir.contains("__rbw_"),
        "LOFT_NO_ADOPT_BUFFER_REUSE: no entry witness is minted"
    );
    assert!(
        !section(&ir, "n_r9").contains("OpRefIsNull(__ref_1(1))"),
        "LOFT_NO_ADOPT_BUFFER_REUSE: the chain callee's buffer stays null"
    );
}

#[test]
fn a_bind_after_an_if_pre_init_adopts() {
    let out = std::env::temp_dir().join("loft_adopt_buffer_reuse_on.rs");
    let rust = emit(&out, &[]);
    // r28: read in a later `if`, so pre-initialised in front of the first — the plain
    // assignment adopts.
    let r28 = section(&rust, "n_r28");
    assert!(
        r28.contains("var_mf = n_scan_mk(cell, 4_i64,"),
        "r28 native: the plain assignment"
    );
    assert!(
        !r28.contains("OpCopyRecord(cell,_src, var_mf"),
        "r28 native: no copy into `mf`"
    );
    // r22: bound inside the `if` alone — the arm's own local (`@FR-B-Scope`, loft#1600), so the
    // bind is its declaration there and adopts the call's buffer.
    let r22 = section(&rust, "n_r22");
    assert!(
        r22.contains("let mut var_mf: DbRef = n_scan_mk(cell, 4_i64,"),
        "r22 native: the plain assignment, declared in the arm"
    );
    assert!(
        !r22.contains("OpCopyRecord(cell,_src, var_mf"),
        "r22 native: no copy into `mf`"
    );
    // A value-record scanner keeps its result in registers beside the entry witness.
    assert!(
        rust.contains(
            "fn n_find_at(cell: &std::cell::UnsafeCell<Stores>, mut var_n: i64, mut var_want: i64) -> (bool, i64)"
        ),
        "find_at: still a value record"
    );
    let _ = std::fs::remove_file(&out);
    let ir = introspect(&[]);
    assert!(
        !section(&ir, "n_r22").contains("CopyRefOrNull"),
        "r22 bytecode: no copy into `mf`"
    );
    let b1_off = introspect(&[("LOFT_NO_ADOPT_FIRST_BIND", "1")]);
    assert!(
        section(&b1_off, "n_r22").contains("CopyRefOrNull"),
        "r22 bytecode under LOFT_NO_ADOPT_FIRST_BIND: the copy is back"
    );
}

#[test]
fn an_inline_container_over_an_adopting_callee_is_released() {
    // r23/r27: `f(…).pts[1]?` binds the call to an inline container; for a callee that
    // returns its promoted local or a chain beside a literal exit, the container is hoisted
    // out of its block like a fresh callee's and released by identity against the buffer.
    let ir = introspect(&[]);
    let via = section(&ir, "n_via_chain");
    assert!(
        via.contains("__ref_p2_1(2):ref(Mk) = null;"),
        "via_chain: the container is hoisted to the statement's scope"
    );
    assert!(
        via.contains("OpFreeRefIfDistinct(__ref_p2_1(2), __ref_1(1))"),
        "via_chain: and released unless it is the pooled buffer"
    );
    for mode in ["--interpret", "--native"] {
        let mut cmd = loft();
        cmd.arg(mode)
            .arg(cells())
            .arg("r23")
            .arg("r27")
            .env("LOFT_NATIVE_LEAK_CHECK", "1")
            .env("LOFT_STRICT_STORES", "1");
        let out = cmd.output().expect("spawn loft");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{mode}: {err}");
        assert!(!err.contains("not freed"), "{mode}: {err}");
    }
}

#[test]
fn a_chain_of_chains_adopts_its_callees_answer() {
    // A pure chain (`fn outer() -> Mk { scan() }`) binds the call to a `__ret_N` native
    // declares up front; that first bind adopts, where a copy freed the caller's pooled
    // buffer the callee had written into.
    let out = std::env::temp_dir().join("loft_adopt_buffer_reuse_chain.rs");
    let guard = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/164-a-chain-of-chains-adopts-its-callees-answer.loft");
    let mut cmd = loft();
    cmd.arg("--native-emit").arg(&out).arg(&guard);
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = std::fs::read_to_string(&out).expect("read the emitted Rust");
    assert!(
        section(&rust, "n_outer")
            .contains("var___ret_1 = n_scan(cell, var_n, var_hit, var___ref_1);"),
        "outer: the chain's answer is adopted"
    );
    let _ = std::fs::remove_file(&out);
    for mode in ["--interpret", "--native"] {
        let mut cmd = loft();
        cmd.arg(mode)
            .arg(&guard)
            .env("LOFT_NATIVE_LEAK_CHECK", "1")
            .env("LOFT_STRICT_STORES", "1");
        let out = cmd.output().expect("spawn loft");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{mode}: {err}");
        assert!(!err.contains("USE AFTER FREE"), "{mode}: {err}");
        assert!(!err.contains("not freed"), "{mode}: {err}");
    }
}

#[test]
fn the_store_census_drops() {
    // Hand-checked 2026-09-17 on the cells as written; a cell edit re-measures both pairs.
    // The pooled counts fell again (246 → 197, 204 → 174) when a literal exit beside a chain
    // began writing the pooled buffer; the unpooled ones are the same, since there the
    // literal mints into the null buffer where it minted a store of its own.  The native pair
    // fell again 174/270 → 169/265 on 2026-09-18 with `@FR-R-LoopRecord` (a record literal
    // bound inside a loop keeps its store across the iterations) — in both arms, so the
    // pool's own drop (96) is unchanged.  Cell r28 (2026-09-22) adds exactly its own share,
    // measured alone: one mint pooled and four without, on each backend.  Re-measured
    // 2026-09-23 with `@FR-B-Scope` (loft#1600): r26 and r28 now bind their local before the
    // block (a read after the block that first binds it is refused), r26 is cheaper for it
    // (10 → 6 on the interpreter, measured against the build before the rule on the same
    // text); r28's author-written null costs 3 mints over the compiler's pre-init it replaces,
    // its rebind taking no pool pairing — loft#1643, fixed before this arc's PR.
    let (i_on, i_off) = (
        store_mints("--interpret", &[]),
        store_mints("--interpret", OFF),
    );
    assert!(
        i_on < i_off,
        "interpret: {i_on} mints pooled, {i_off} without"
    );
    let (n_on, n_off) = (store_mints("--native", &[]), store_mints("--native", OFF));
    assert!(n_on < n_off, "native: {n_on} mints pooled, {n_off} without");
    assert_eq!(
        (i_on, i_off, n_on, n_off),
        (197, 303, 173, 269),
        "mints (interpret on, off, native on, off)"
    );
}
