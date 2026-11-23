// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN203 rust-writer: counts the Rust writer's pooled mints — its `--native` runs as `--native-rust`.
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
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    // The tests run in parallel on one source file, and its program cache is shared: an
    // `introspect` that read another process's half-written entry listed a truncated program.
    cmd.env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_ADOPT_BUFFER_REUSE")
        .env_remove("LOFT_NO_ADOPT_FIRST_BIND");
    cmd
}

fn with_env(cmd: &mut Command, env: &[(&str, &str)]) {
    for (k, v) in env {
        cmd.env(k, v);
    }
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

const OFF: &[(&str, &str)] = &[("LOFT_NO_ADOPT_BUFFER_REUSE", "1")];

#[test]
fn the_entry_witness_guards_the_promoted_buffer() {
    let ir = introspect(&[]);
    // The rebind of a promoted buffer (D-own-43): the snapshot, and the interpreter's
    // displaced free guarded by it, ahead of the call — where the rebind does not stand
    // directly before the tail (`_apart`).  Directly before it, the tail local is handed the
    // return buffer: the call writes `cv`, the caller's buffer, itself.
    let lit = section(&ir, "n_render_lit_then_call");
    assert!(
        lit.contains(
            "cv(0):ref(Canvas) = n_alloc_canvas(4i32, 5i32, OpGetInt(p(0), 0i32), cv(0));"
        ),
        "render_lit_then_call: the rebind before the tail is handed the return buffer"
    );
    let r1 = section(&ir, "n_render_lit_then_call_apart");
    assert!(
        r1.contains("__rbw_cv(0):ref(Canvas)[\"__rbw_cv\"] = OpRefAlias(cv(0));"),
        "render_lit_then_call_apart: the entry witness is minted"
    );
    // Two spellings of the same guard: one witness (`FreeRefIfDistinct`) or, since the
    // hidden buffer of the call became a witness too (`@FR-O-Buffer`, the pooled-buffer
    // rebind), the entry beside it (`FreeRefUnlessEntry`, which declines on either).
    let free = [
        r1.find("FreeRefIfDistinct(placeholder"),
        r1.find("FreeRefUnlessEntry(placeholder"),
    ]
    .into_iter()
    .flatten()
    .min()
    .expect("render_lit_then_call_apart: the rebind's displaced free is guarded");
    let call = r1
        .find("fn=n_alloc_canvas)")
        .expect("render_lit_then_call_apart calls alloc_canvas");
    assert!(
        free < call,
        "render_lit_then_call_apart: the guarded free precedes the call"
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
    // The switch turns off the POOL alone: the entry witness stays, because a rebind of the
    // promoted buffer frees what it displaces only through it (`bind_lowering`).
    assert!(
        ir.contains("__rbw_"),
        "LOFT_NO_ADOPT_BUFFER_REUSE: the entry witness is still minted"
    );
    assert!(
        !section(&ir, "n_r9").contains("OpRefIsNull(__ref_1(1))"),
        "LOFT_NO_ADOPT_BUFFER_REUSE: the chain callee's buffer stays null"
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
    // And with the adopt switched off (`(R-Switch)`'s off form copies): the temp still OWNS
    // what the call answered, so it is still hoisted and released (loft#1704).
    for env in [&[][..], &[("LOFT_NO_ADOPT_FIRST_BIND", "1")][..]] {
        for mode in ["--interpret", "--native"] {
            let mut cmd = loft();
            cmd.arg(mode)
                .arg(cells())
                .arg("r23")
                .arg("r27")
                .env("LOFT_NATIVE_LEAK_CHECK", "1")
                .env("LOFT_STRICT_STORES", "1");
            with_env(&mut cmd, env);
            let out = cmd.output().expect("spawn loft");
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success(), "{mode} {env:?}: {err}");
            assert!(!err.contains("not freed"), "{mode} {env:?}: {err}");
        }
    }
}
