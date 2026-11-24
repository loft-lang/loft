// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RefillBuffer`, PINNED on its cells (`tests/scripts/a-return-buffer-refills-the-store-a-rebind-released.loft`):
//! which callees mint their return buffer with `OpDatabaseRefill` and empty its vector fields
//! in place, that the text-holding type declines, and that both switch states answer the same
//! and leave no store behind on `--native`.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-return-buffer-refills-the-store-a-rebind-released.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> (String, String, bool) {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_REFILL_BUFFER")
        .env_remove("LOFT_NO_SWAP_REBIND")
        .env_remove("LOFT_NO_REFILL_IN_PLACE");
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
    for env in [&[][..], &[("LOFT_NO_REFILL_BUFFER", "1")][..]] {
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
