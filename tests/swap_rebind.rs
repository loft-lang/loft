// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-H-SwapRebind`, PINNED on its cells (`tests/scripts/a-rebind-exchanges-into-a-destination-it-does-not-reset.loft`):
//! the exchange replaces the COPY a rebind's copy-or-adopt arm makes.  The rustc writer takes
//! that arm only where `bind_lowering::re_bind` copies or joins, and binds these cells' rebinds
//! plainly (the fresh answer adopted, nothing copied), so the sites are counted on the arm
//! itself: `LOFT_NATIVE_OWN_CALL_BIND=1` gives every reassignment the copy-or-adopt.  The
//! cells answer the same with the exchange on and off and on either route, and no store is
//! left behind — a released store the exchange kept instead of freeing reads as a leak here
//! and nowhere else.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-rebind-exchanges-into-a-destination-it-does-not-reset.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> (String, String, bool) {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_SWAP_REBIND")
        .env_remove("LOFT_NO_STORE_SWAP")
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
        &[][..],
        &[("LOFT_NO_SWAP_REBIND", "1")][..],
        &[("LOFT_NATIVE_OWN_CALL_BIND", "1")][..],
    ] {
        let mut env = env.to_vec();
        env.push(("LOFT_NATIVE_LEAK_CHECK", "1"));
        let (out, err, ok) = loft(&["--native", &cells()], &env);
        assert!(ok && out.contains("done"), "{env:?}:\n{out}\n{err}");
        assert!(
            !err.contains("not freed"),
            "{env:?} left stores behind:\n{err}"
        );
    }
}
