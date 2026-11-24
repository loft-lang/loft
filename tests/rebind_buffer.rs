// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RebindBuffer`, PINNED on its cells (`tests/scripts/a-rebind-hands-the-displaced-store-to-the-call.loft`):
//! which rebinds hand the displaced store to the call's hidden buffer, that both switch states
//! answer the same and leave no store behind on `--native`, and that strict stores see no
//! reference into a released store.  The hand-over is part of a rebind's copy-or-adopt arm;
//! the rustc writer takes that arm only where `bind_lowering::re_bind` copies or joins and binds
//! these cells' rebinds plainly, so the sites are counted on the arm itself:
//! `LOFT_NATIVE_OWN_CALL_BIND=1` gives every reassignment the copy-or-adopt.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-rebind-hands-the-displaced-store-to-the-call.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> (String, String, bool) {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_REBIND_BUFFER")
        .env_remove("LOFT_NO_SWAP_REBIND")
        .env_remove("LOFT_NO_REFILL_BUFFER")
        .env_remove("LOFT_NATIVE_OWN_CALL_BIND");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn cells() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CELLS)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn the_cells_answer_the_same_and_free_every_store() {
    for env in [
        &[("LOFT_NATIVE_LEAK_CHECK", "1")][..],
        &[
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
            ("LOFT_NO_REBIND_BUFFER", "1"),
        ][..],
        &[("LOFT_STRICT_STORES", "1"), ("LOFT_POISON", "1")][..],
        &[
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
            ("LOFT_NATIVE_OWN_CALL_BIND", "1"),
        ][..],
    ] {
        let (out, err, ok) = loft(&["--native", &cells()], env);
        assert!(ok, "{env:?}:\n{out}\n{err}");
        assert!(
            !err.contains("not freed"),
            "{env:?} left stores behind:\n{err}"
        );
    }
}
